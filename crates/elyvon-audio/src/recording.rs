//! Multitrack input capture: RT tap -> bounded SPSC -> BWF/WAV disk worker.
//! Files retain valid checkpointed headers and a recovery journal until commit.
use crate::rt::AtomicF32;
use crate::transport::PlayState;
use elyvon_project::TrackId;
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ts_rs::TS;

pub const MAX_INPUTS: usize = 32;
const QUEUE: usize = 8192;
const HEADER: usize = 668;

#[derive(Debug, Clone)]
pub struct CaptureRoute {
    pub track: TrackId,
    pub input: u16,
    pub channels: u16,
    pub armed: bool,
    pub monitor: bool,
    pub gain: f32,
}
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub rate: u32,
    pub input_channels: u16,
    pub start_frame: u64,
    pub punch: Option<(u64, u64)>,
    pub cycle: Option<(u64, u64)>,
    pub compensation_frames: i64,
    pub monitor_only: bool,
    /// An optional disk quota, useful for deterministic fault-injection tests.
    pub max_bytes: Option<u64>,
}
#[derive(Debug, Clone, Copy)]
struct InputFrame {
    frame: u64,
    cycle: u64,
    samples: [f32; MAX_INPUTS],
}
#[derive(Debug, Default)]
pub struct CaptureHealth {
    pub overflow: AtomicBool,
    pub disconnected: AtomicBool,
    pub clipped: AtomicU64,
    pub peak: AtomicF32,
    pub input_latency_frames: AtomicU64,
}

#[derive(Debug)]
pub struct CaptureTap {
    tx: Producer<InputFrame>,
    pub health: Arc<CaptureHealth>,
    config: CaptureConfig,
    routes: Vec<CaptureRoute>,
    monitor: Option<crate::engine::MonitorInput>,
    cursor: Option<(i128, u64)>,
}
impl CaptureTap {
    pub fn input_channels(&self) -> u16 {
        self.config.input_channels
    }

    /// `position` is the captured first frame in the output timeline (after
    /// IO-clock mapping). Compensation is applied once here, never by the UI.
    /// No allocation, locks, logs, IO or blocking calls.
    pub fn process(&mut self, input: &[f32], position: u64, state: PlayState, cycle: u64) {
        let channels = usize::from(self.config.input_channels);
        if channels == 0 || self.health.disconnected.load(Ordering::Relaxed) {
            return;
        }
        if state != PlayState::Playing {
            self.cursor = None;
        }
        // Latch the hardware clock once, then count integer input frames. Host
        // callback jitter must never create gaps or a take per callback.
        let (mut frame, mut lap) = self.cursor.unwrap_or((
            i128::from(position) - i128::from(self.config.compensation_frames),
            cycle,
        ));
        for data in input.chunks_exact(channels) {
            if let Some((start, end)) = self.config.cycle
                && frame >= i128::from(end)
                && position < end
            {
                let extra = frame - i128::from(end);
                let length = i128::from(end - start);
                lap = lap.saturating_add(1 + (extra / length) as u64);
                frame = i128::from(start) + extra % length;
            }
            let mut packet = InputFrame {
                frame: u64::try_from(frame).unwrap_or(0),
                cycle: lap,
                samples: [0.0; MAX_INPUTS],
            };
            let mut peak = 0.0f32;
            for (out, &s) in packet.samples.iter_mut().zip(data) {
                *out = if s.is_finite() { s } else { 0.0 };
                peak = peak.max(out.abs());
            }
            if peak > self.health.peak.load() {
                self.health.peak.store(peak);
            }
            if peak >= 1.0 {
                self.health.clipped.fetch_add(1, Ordering::Relaxed);
            }
            if let Some(tx) = &mut self.monitor {
                let mut pair = [0.0; 2];
                for route in self.routes.iter().filter(|r| r.monitor) {
                    let ch = usize::from(route.input);
                    pair[0] += packet.samples[ch] * route.gain;
                    pair[1] += packet.samples[ch + usize::from(route.channels - 1)] * route.gain;
                }
                let _ = tx.push(pair);
            }
            let eligible = state == PlayState::Playing
                && frame >= 0
                && (lap > 0 || frame >= i128::from(self.config.start_frame))
                && self
                    .config
                    .punch
                    .is_none_or(|(a, b)| frame >= i128::from(a) && frame < i128::from(b));
            if eligible
                && !self.config.monitor_only
                && !self.health.overflow.load(Ordering::Relaxed)
                && self.tx.push(packet).is_err()
            {
                // Fail closed: never silently compress time or drop samples.
                self.health.overflow.store(true, Ordering::Release);
            }
            frame = frame.saturating_add(1);
        }
        if state == PlayState::Playing {
            self.cursor = Some((frame, lap));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RecordingState {
    Recording,
    Monitoring,
    Finalized,
    Cancelled,
    Interrupted,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecordedTake {
    pub track: TrackId,
    pub path: String,
    pub sample_rate: u32,
    pub channels: u16,
    #[ts(type = "number")]
    pub start_frame: u64,
    #[ts(type = "number")]
    pub frames: u64,
    #[ts(type = "number")]
    pub cycle: u64,
    #[ts(type = "number")]
    pub recorded_at: u64,
    pub recoverable: bool,
}
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecordingReport {
    pub state: RecordingState,
    pub takes: Vec<RecordedTake>,
    pub error: Option<String>,
    pub input_peak: f32,
    #[ts(type = "number")]
    pub clipped_frames: u64,
    pub overflow: bool,
}
#[derive(Debug)]
pub struct RecordingWriter {
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    pub health: Arc<CaptureHealth>,
    worker: Option<JoinHandle<RecordingReport>>,
}
impl Drop for RecordingWriter {
    fn drop(&mut self) {
        self.health.disconnected.store(true, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
impl RecordingWriter {
    pub fn start(
        dir: &Path,
        config: CaptureConfig,
        routes: Vec<CaptureRoute>,
        monitor: Option<crate::engine::MonitorInput>,
    ) -> Result<(CaptureTap, Self), String> {
        if config.rate == 0
            || config.input_channels == 0
            || usize::from(config.input_channels) > MAX_INPUTS
            || routes.is_empty()
            || routes.len() > 32
            || config.compensation_frames.unsigned_abs() > u64::from(config.rate) * 10
            || config.punch.is_some_and(|(a, b)| a >= b)
            || config.cycle.is_some_and(|(a, b)| a >= b)
            || routes.iter().any(|r| {
                !(1..=2).contains(&r.channels)
                    || r.input.saturating_add(r.channels) > config.input_channels
                    || !r.gain.is_finite()
            })
        {
            return Err("invalid capture routing, rate, range or compensation".into());
        }
        if !config.monitor_only && !routes.iter().any(|r| r.armed) {
            return Err("arm at least one audio track".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        // Fail before opening the input stream if the destination is unwritable.
        let probe = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        drop(probe);
        let (tx, rx) = RingBuffer::new(QUEUE);
        let health = Arc::new(CaptureHealth::default());
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let tap = CaptureTap {
            tx,
            health: Arc::clone(&health),
            config: config.clone(),
            routes: routes.clone(),
            monitor,
            cursor: None,
        };
        let (h, s, c) = (Arc::clone(&health), Arc::clone(&stop), Arc::clone(&cancel));
        let dir = dir.to_path_buf();
        let worker = std::thread::Builder::new()
            .name("elyvon-record-writer".into())
            .spawn(move || write_worker(&dir, config, routes, rx, h, s, c))
            .map_err(|e| e.to_string())?;
        Ok((
            tap,
            Self {
                stop,
                cancel,
                health,
                worker: Some(worker),
            },
        ))
    }
    pub fn finish(mut self, cancel: bool) -> RecordingReport {
        self.cancel.store(cancel, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        self.worker
            .take()
            .and_then(|w| w.join().ok())
            .unwrap_or_else(|| RecordingReport {
                state: RecordingState::Failed,
                takes: vec![],
                error: Some(
                    "recording worker stopped unexpectedly; partial files are recoverable".into(),
                ),
                input_peak: self.health.peak.load(),
                clipped_frames: self.health.clipped.load(Ordering::Relaxed),
                overflow: self.health.overflow.load(Ordering::Relaxed),
            })
    }
}

struct TakeFile {
    file: BufWriter<File>,
    meta: RecordedTake,
    journal: PathBuf,
}
impl TakeFile {
    fn new(dir: &Path, route: &CaptureRoute, p: &InputFrame, rate: u32) -> std::io::Result<Self> {
        let unique = tempfile::Builder::new()
            .prefix(&format!("{}-take-", route.track))
            .suffix(".partial.wav")
            .tempfile_in(dir)?;
        let (file, path) = unique.keep().map_err(|e| e.error)?;
        let meta = RecordedTake {
            track: route.track,
            path: path.display().to_string(),
            sample_rate: rate,
            channels: route.channels,
            start_frame: p.frame,
            frames: 0,
            cycle: p.cycle,
            recorded_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            recoverable: true,
        };
        let journal = path.with_extension("json");
        let mut take = Self {
            file: BufWriter::with_capacity(256 << 10, file),
            meta,
            journal,
        };
        take.checkpoint()?;
        Ok(take)
    }
    fn checkpoint(&mut self) -> std::io::Result<()> {
        self.file.flush()?;
        let header = wav_header(&self.meta)?;
        let file = self.file.get_mut();
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header)?;
        file.seek(SeekFrom::End(0))?;
        file.sync_data()?;
        let data = serde_json::to_vec_pretty(&self.meta).map_err(std::io::Error::other)?;
        elyvon_core::fs::atomic_write(&self.journal, &data)
    }
    fn finish(mut self, commit: bool) -> std::io::Result<RecordedTake> {
        self.checkpoint()?;
        if commit {
            let from = PathBuf::from(&self.meta.path);
            let dest = from.with_file_name(
                from.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .replace(".partial.wav", ".wav"),
            );
            drop(self.file);
            // Unique take names never overwrite an existing file.
            if dest.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "take destination exists",
                ));
            }
            std::fs::hard_link(&from, &dest)?;
            std::fs::remove_file(&from)?;
            self.meta.path = dest.display().to_string();
            self.meta.recoverable = false;
            elyvon_core::fs::atomic_write(
                &self.journal,
                &serde_json::to_vec_pretty(&self.meta).map_err(std::io::Error::other)?,
            )?;
        }
        Ok(self.meta)
    }
}
fn write_worker(
    dir: &Path,
    config: CaptureConfig,
    routes: Vec<CaptureRoute>,
    mut rx: Consumer<InputFrame>,
    health: Arc<CaptureHealth>,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> RecordingReport {
    let mut active: Vec<Option<TakeFile>> = routes.iter().map(|_| None).collect();
    let mut report = RecordingReport {
        state: RecordingState::Recording,
        takes: vec![],
        error: None,
        input_peak: 0.0,
        clipped_frames: 0,
        overflow: false,
    };
    let mut bytes = 0u64;
    let result = (|| -> std::io::Result<()> {
        while !stop.load(Ordering::Acquire) || !rx.is_empty() {
            let packet = match rx.pop() {
                Ok(p) => p,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
            };
            for (i, route) in routes.iter().enumerate().filter(|(_, r)| r.armed) {
                let split = active[i].as_ref().is_some_and(|t| {
                    t.meta.cycle != packet.cycle
                        || t.meta.start_frame + t.meta.frames != packet.frame
                        || t.meta.frames.saturating_mul(u64::from(t.meta.channels) * 4)
                            > u64::from(u32::MAX) - HEADER as u64 - 256
                });
                if split && let Some(t) = active[i].take() {
                    report.takes.push(t.finish(true)?);
                }
                if active[i].is_none() {
                    active[i] = Some(TakeFile::new(dir, route, &packet, config.rate)?);
                }
                let take = active[i]
                    .as_mut()
                    .ok_or_else(|| std::io::Error::other("missing take writer"))?;
                for ch in route.input..route.input + route.channels {
                    if config.max_bytes.is_some_and(|limit| bytes + 4 > limit) {
                        return Err(std::io::Error::other("disk quota exhausted"));
                    }
                    take.file
                        .write_all(&packet.samples[usize::from(ch)].to_le_bytes())?;
                    bytes += 4;
                }
                take.meta.frames += 1;
                if take.meta.frames % u64::from(config.rate) == 0 {
                    take.checkpoint()?;
                }
            }
        }
        Ok(())
    })();
    let interrupted =
        health.overflow.load(Ordering::Acquire) || health.disconnected.load(Ordering::Acquire);
    let cancelled = cancel.load(Ordering::Acquire);
    let commit = result.is_ok() && !interrupted && !cancelled;
    if let Err(e) = result {
        report.error = Some(e.to_string());
        health.disconnected.store(true, Ordering::Release);
    }
    for take in active.into_iter().flatten() {
        let fallback = take.meta.clone();
        match take.finish(commit) {
            Ok(t) => report.takes.push(t),
            Err(e) => {
                report.error = Some(e.to_string());
                report.takes.push(fallback);
            }
        }
    }
    report.state = if report.error.is_some() {
        RecordingState::Failed
    } else if cancelled {
        RecordingState::Cancelled
    } else if interrupted {
        RecordingState::Interrupted
    } else {
        RecordingState::Finalized
    };
    if interrupted && report.error.is_none() {
        report.error=Some("capture interrupted by an input disconnection or disk backpressure; recover the partial takes".into());
    }
    report.input_peak = health.peak.load();
    report.clipped_frames = health.clipped.load(Ordering::Relaxed);
    report.overflow = health.overflow.load(Ordering::Relaxed);
    report
}
fn wav_header(take: &RecordedTake) -> std::io::Result<Vec<u8>> {
    let size = take
        .frames
        .checked_mul(u64::from(take.channels) * 4)
        .filter(|s| *s + HEADER as u64 - 8 <= u64::from(u32::MAX))
        .ok_or_else(|| std::io::Error::other("WAV size limit"))? as u32;
    let mut header = crate::render::encode_wav_f32(&[], take.channels, take.sample_rate);
    let mut bext = vec![0u8; 610];
    bext[..4].copy_from_slice(b"bext");
    bext[4..8].copy_from_slice(&602u32.to_le_bytes());
    bext[8..8 + 19].copy_from_slice(b"Elyvon Studio take ");
    bext[8 + 256..8 + 269].copy_from_slice(b"Elyvon Studio");
    if take.recorded_at > 253402300799 {
        return Err(std::io::Error::other("BWF timestamp exceeds year 9999"));
    }
    let days = (take.recorded_at / 86400) as i64 + 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let date = format!("{year:04}-{month:02}-{day:02}");
    let seconds = take.recorded_at % 86400;
    let time = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    );
    bext[8 + 320..8 + 330].copy_from_slice(date.as_bytes());
    bext[8 + 330..8 + 338].copy_from_slice(time.as_bytes());
    let reference = (take.recorded_at % 86400) * u64::from(take.sample_rate);
    bext[8 + 338..8 + 346].copy_from_slice(&reference.to_le_bytes());
    bext[8 + 346..8 + 348].copy_from_slice(&1u16.to_le_bytes());
    header.splice(12..12, bext);
    header[4..8].copy_from_slice(&(HEADER as u32 - 8 + size).to_le_bytes());
    header[HEADER - 4..HEADER].copy_from_slice(&size.to_le_bytes());
    header[HEADER - 12..HEADER - 8].copy_from_slice(&(take.frames as u32).to_le_bytes());
    Ok(header)
}

/// Recover a journalled partial take after a crash or failed capture. The
/// complete channel-aligned bytes on disk are authoritative, never the last
/// checkpoint's length. This function never overwrites a finalized take.
pub fn recover_take(journal: &Path) -> Result<RecordedTake, String> {
    let bytes = std::fs::read(journal).map_err(|e| e.to_string())?;
    let mut take: RecordedTake = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if !take.recoverable || !(1..=2).contains(&take.channels) || take.sample_rate == 0 {
        return Err("not a recoverable recording".into());
    }
    let path = PathBuf::from(&take.path);
    if path != journal.with_extension("wav") {
        return Err("journal and take paths differ".into());
    }
    if !path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(".partial.wav"))
    {
        return Err("not a partial take".into());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len < HEADER as u64 {
        return Err("partial file has no complete WAV header".into());
    }
    take.frames = (len - HEADER as u64) / (u64::from(take.channels) * 4);
    if take.frames == 0 {
        return Err("partial take contains no complete frames".into());
    }
    file.set_len(HEADER as u64 + take.frames * u64::from(take.channels) * 4)
        .map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.write_all(&wav_header(&take)?))
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    let take_file = TakeFile {
        file: BufWriter::new(file),
        meta: take,
        journal: journal.to_path_buf(),
    };
    take_file.finish(true).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_stalled_disk_queue_fails_closed_and_leaves_a_recoverable_take() {
        let dir = tempfile::tempdir().expect("temp directory");
        let config = CaptureConfig {
            rate: 48000,
            input_channels: 1,
            start_frame: 0,
            punch: None,
            cycle: None,
            compensation_frames: 0,
            monitor_only: false,
            max_bytes: None,
        };
        let routes = vec![CaptureRoute {
            track: TrackId::new(),
            input: 0,
            channels: 1,
            armed: true,
            monitor: false,
            gain: 1.0,
        }];
        let (tx, rx) = RingBuffer::new(1);
        let health = Arc::new(CaptureHealth::default());
        let mut tap = CaptureTap {
            tx,
            health: Arc::clone(&health),
            config: config.clone(),
            routes: routes.clone(),
            monitor: None,
            cursor: None,
        };
        tap.process(&[0.1, 0.2, 0.3], 0, PlayState::Playing, 0);
        assert!(health.overflow.load(Ordering::Acquire));
        drop(tap);
        let report = write_worker(
            dir.path(),
            config,
            routes,
            rx,
            health,
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(report.state, RecordingState::Interrupted);
        assert!(report.overflow);
        assert_eq!(report.takes[0].frames, 1);
        assert!(report.takes[0].recoverable);
    }
}
