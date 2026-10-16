//! Offline rendering through the real-time engine code path, WAV export and a
//! diagnostic report (levels per channel and per track, checksum).

use std::path::Path;
use std::sync::Arc;

use elyvon_core::Ticks;
use elyvon_project::{MediaId, Project};
use serde::Serialize;
use ts_rs::TS;

use crate::engine::{self, MidiSink, NoMidi};
use crate::media::MediaBuffer;
use crate::plan::build_plan;
use crate::transport::TransportCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRequest {
    pub sample_rate: u32,
    pub start_tick: Ticks,
    /// `None` renders to the end of the last region.
    pub end_tick: Option<Ticks>,
    pub block_frames: usize,
    pub metronome: bool,
}

impl RenderRequest {
    pub fn whole_song(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            start_tick: 0,
            end_tick: None,
            block_frames: 512,
            metronome: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrackLevel {
    pub name: String,
    pub peak: f32,
    pub rms: f64,
}

/// Diagnostic summary written next to an exported render.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenderReport {
    pub sample_rate: u32,
    #[ts(type = "number")]
    pub frames: u64,
    pub seconds: f64,
    #[ts(type = "number")]
    pub start_tick: u64,
    #[ts(type = "number")]
    pub end_tick: u64,
    #[ts(type = "number")]
    pub block_frames: u64,
    /// Linear peak and RMS per output channel (L, R).
    pub peak: [f32; 2],
    pub rms: [f64; 2],
    pub tracks: Vec<TrackLevel>,
    /// Audio clips (after muting) that had decoded media.
    #[ts(type = "number")]
    pub clips: u64,
    /// Audio regions that stayed silent because their file is unavailable.
    #[ts(type = "number")]
    pub missing_media: u64,
    /// Nonzero means this diagnostic render is incomplete and cannot be exported.
    #[ts(type = "number")]
    pub streaming_errors: u64,
    /// FNV-1a 64 over the f32 sample bits, hex.
    pub checksum: String,
}

#[derive(Debug, Clone)]
pub struct Rendered {
    /// Interleaved stereo.
    pub samples: Vec<f32>,
    pub report: RenderReport,
}

impl Rendered {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn channel(&self, ch: usize) -> Vec<f32> {
        self.samples.iter().skip(ch).step_by(2).copied().collect()
    }
}

/// Renders a project offline with the same [`engine::Engine`] used for
/// playback and collects the samples. Output is independent of
/// `block_frames` (tested).
pub fn render(
    project: &Project,
    media: &dyn Fn(MediaId) -> Option<Arc<MediaBuffer>>,
    request: RenderRequest,
    midi: Option<&mut dyn MidiSink>,
) -> Rendered {
    let mut samples = Vec::new();
    let report = render_stream(project, media, request, midi, &mut |block| {
        samples.extend_from_slice(block)
    });
    Rendered { samples, report }
}

/// Like [`render`] but hands each interleaved stereo block to `sink` instead
/// of keeping the whole render in memory.
pub fn render_stream(
    project: &Project,
    media: &dyn Fn(MediaId) -> Option<Arc<MediaBuffer>>,
    request: RenderRequest,
    midi: Option<&mut dyn MidiSink>,
    sink: &mut dyn FnMut(&[f32]),
) -> RenderReport {
    let rate = request.sample_rate;
    let mut plan = build_plan(project, media, rate, 1);
    let _offline = OfflinePages::new(&plan);
    plan.metronome = request.metronome;
    plan.cycle = None;
    let start = plan.map.sample_at_tick(request.start_tick, rate);
    let end_tick = request.end_tick.unwrap_or_else(|| project.content_end());
    let end = plan
        .map
        .sample_at_tick(end_tick.max(request.start_tick), rate);
    let stats = plan.stats;
    let block = request.block_frames.max(1);
    let (mut handle, mut engine) = engine::create(rate, block, plan);
    engine.set_clip_output(false);
    engine.enable_measurement();
    handle.send(TransportCommand::Play {
        from_tick: Some(request.start_tick),
        count_in: false,
        pre_roll: false,
    });
    let frames = (end - start) as usize;
    let mut no_midi = NoMidi;
    let midi: &mut dyn MidiSink = match midi {
        Some(m) => m,
        None => &mut no_midi,
    };
    let mut buf = vec![0.0f32; block * 2];
    let mut peak = [0.0f32; 2];
    let mut sum = [0.0f64; 2];
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut done = 0usize;
    let mut streaming_errors = 0;
    while done < frames {
        let n = block.min(frames - done);
        let chunk = &mut buf[..n * 2];
        // Offline rendering may wait for IO; the RT callback never does.
        if engine.prepare_media(start + done as u64, n).is_err() {
            streaming_errors += 1;
        }
        let before = handle.shared().stats().streaming_underruns;
        engine.process(chunk, 2, midi);
        if handle.shared().stats().streaming_underruns > before {
            streaming_errors += 1;
        }
        for frame in chunk.as_chunks::<2>().0 {
            for ch in 0..2 {
                let s = frame[ch];
                peak[ch] = peak[ch].max(s.abs());
                sum[ch] += f64::from(s) * f64::from(s);
                for b in s.to_bits().to_le_bytes() {
                    hash ^= u64::from(b);
                    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        sink(chunk);
        done += n;
    }
    let n = frames.max(1) as f64;
    let measures = engine.measurements().unwrap_or(&[]);
    let tracks = project
        .tracks
        .iter()
        .zip(measures)
        .map(|(t, m)| TrackLevel {
            name: t.name.clone(),
            peak: m.peak,
            rms: (m.sum_squares / (2.0 * m.frames.max(1) as f64)).sqrt(),
        })
        .collect();
    drop(engine);
    handle.poll();
    RenderReport {
        sample_rate: rate,
        frames: frames as u64,
        seconds: frames as f64 / f64::from(rate),
        start_tick: request.start_tick,
        end_tick,
        block_frames: block as u64,
        peak,
        rms: [(sum[0] / n).sqrt(), (sum[1] / n).sqrt()],
        tracks,
        clips: stats.clips as u64,
        missing_media: stats.missing_media as u64,
        streaming_errors,
        checksum: format!("{hash:016x}"),
    }
}

/// Encodes interleaved `f32` samples as a 32-bit float WAVE file.
pub fn encode_wav_f32(samples: &[f32], channels: u16, rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 4) as u32;
    let frames = (samples.len() / usize::from(channels.max(1))) as u32;
    let mut out = Vec::with_capacity(58 + samples.len() * 4);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(4 + 26 + 12 + 8 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&18u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // WAVE_FORMAT_IEEE_FLOAT
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 4).to_le_bytes());
    out.extend_from_slice(&(channels * 4).to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    out.extend_from_slice(b"fact");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&frames.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Writes a render and its JSON report (`<path>.json`) atomically.
pub fn export(path: &Path, rendered: &Rendered) -> std::io::Result<std::path::PathBuf> {
    if rendered.report.streaming_errors != 0 {
        return Err(std::io::Error::other(
            "render aborted: disk-cache reads failed",
        ));
    }
    let wav = encode_wav_f32(&rendered.samples, 2, rendered.report.sample_rate);
    elyvon_core::fs::atomic_write(path, &wav)?;
    let mut report_path = path.as_os_str().to_owned();
    report_path.push(".json");
    let report_path = std::path::PathBuf::from(report_path);
    let json = serde_json::to_vec_pretty(&rendered.report).map_err(std::io::Error::other)?;
    elyvon_core::fs::atomic_write(&report_path, &json)?;
    Ok(report_path)
}

// Offline callers prepare exactly the next block. Speculative reads must not
// evict pages from other clips in that prepared working set. Concurrent live
// playback should use independent MediaBuffers (the desktop export guards it).
struct OfflinePages(Vec<(Arc<MediaBuffer>, bool)>);
impl OfflinePages {
    fn new(plan: &crate::plan::PlaybackPlan) -> Self {
        let mut held: Vec<(Arc<MediaBuffer>, bool)> = Vec::new();
        for track in &plan.tracks {
            for segment in &track.segments {
                if let Some(stream) = &segment.media.stream
                    && !held.iter().any(|(m, _)| Arc::ptr_eq(m, &segment.media))
                {
                    let old = stream.set_read_ahead(false);
                    held.push((Arc::clone(&segment.media), old));
                }
            }
        }
        Self(held)
    }
}
impl Drop for OfflinePages {
    fn drop(&mut self) {
        for (media, old) in &self.0 {
            if let Some(stream) = &media.stream {
                stream.set_read_ahead(*old);
            }
        }
    }
}

/// Bounded-memory WAV export. Samples go directly to a temporary file; source
/// files and an existing successful export remain intact on read/write failure.
pub fn export_project(
    path: &Path,
    project: &Project,
    media: &dyn Fn(MediaId) -> Option<Arc<MediaBuffer>>,
    request: RenderRequest,
) -> std::io::Result<RenderReport> {
    use std::io::{Seek, SeekFrom, Write};
    let map = project
        .transport
        .tempo_map()
        .map_err(std::io::Error::other)?;
    let start = map.sample_at_tick(request.start_tick, request.sample_rate);
    let end = map.sample_at_tick(
        request.end_tick.unwrap_or_else(|| project.content_end()),
        request.sample_rate,
    );
    let frames = end.saturating_sub(start);
    let size = frames
        .checked_mul(8)
        .filter(|n| *n <= u64::from(u32::MAX) - 58)
        .ok_or_else(|| {
            std::io::Error::other(
                "diagnostic WAV exceeds the 4 GiB format limit; export a shorter range",
            )
        })? as u32;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&encode_wav_f32(&[], 2, request.sample_rate))?;
    let mut bytes = Vec::with_capacity(request.block_frames.max(1).saturating_mul(8));
    let mut failure = None;
    let report = render_stream(project, media, request, None, &mut |block| {
        if failure.is_some() {
            return;
        }
        bytes.clear();
        for sample in block {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        if let Err(e) = temp.write_all(&bytes) {
            failure = Some(e);
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    if report.streaming_errors != 0 {
        return Err(std::io::Error::other(
            "disk-cache reads failed; previous export preserved",
        ));
    }
    let mut header = encode_wav_f32(&[], 2, request.sample_rate);
    header[4..8].copy_from_slice(&(50 + size).to_le_bytes());
    header[46..50].copy_from_slice(&(frames as u32).to_le_bytes());
    header[54..58].copy_from_slice(&size.to_le_bytes());
    temp.seek(SeekFrom::Start(0))?;
    temp.write_all(&header)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    let mut report_path = path.as_os_str().to_owned();
    report_path.push(".json");
    elyvon_core::fs::atomic_write(
        Path::new(&report_path),
        &serde_json::to_vec_pretty(&report).map_err(std::io::Error::other)?,
    )?;
    Ok(report)
}
