//! Media pool runtime: decoded, resampled, analysed audio shared with the
//! engine through immutable `Arc<MediaBuffer>`s.
//!
//! Source files are read-only. Small sources use resident buffers; larger
//! sources decode/resample in bounded packets to a PCM disk cache. Workers
//! prepare atomic pages, and the callback never reads or waits on a file.

pub mod decode;
pub mod peaks;
pub mod stream;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use elyvon_project::{MediaAsset, MediaId, MediaInfo};
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Fft, FixedSync, Resampler};
use serde::Serialize;
use ts_rs::TS;

pub mod repair;
pub use decode::{Decoded, SUPPORTED_EXTENSIONS, decode_file, fingerprint};
pub use peaks::PeakPyramid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    NotFound(String),
    Io(String),
    Unsupported(String),
    Decode(String),
    Empty,
    TooLarge,
    Resample(String),
}

impl MediaError {
    pub fn from_io(path: &Path, e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            Self::NotFound(path.display().to_string())
        } else {
            Self::Io(format!("{}: {e}", path.display()))
        }
    }

    /// Stable code for the UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "missing",
            Self::Io(_) => "io",
            Self::Unsupported(_) => "unsupported",
            Self::Decode(_) => "decode",
            Self::Empty => "empty",
            Self::TooLarge => "tooLarge",
            Self::Resample(_) => "resample",
        }
    }
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "file not found: {p}"),
            Self::Io(m) => write!(f, "could not read the file: {m}"),
            Self::Unsupported(m) => write!(f, "{m}"),
            Self::Decode(m) => write!(f, "the file could not be decoded: {m}"),
            Self::Empty => f.write_str("the file contains no audio"),
            Self::TooLarge => {
                f.write_str("the audio exceeds the configured memory or disk-cache format limit")
            }
            Self::Resample(m) => write!(f, "sample-rate conversion failed: {m}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// Decoded audio prepared for one engine sample rate. Immutable once built.
#[derive(Debug)]
pub struct MediaBuffer {
    /// Measured properties of the source file.
    pub info: MediaInfo,
    /// Rate of `planes` (the engine rate).
    pub rate: u32,
    /// One or two channels (sources with more channels keep the first two).
    pub planes: Vec<Vec<f32>>,
    /// Waveform peaks at the source rate.
    pub peaks: PeakPyramid,
    pub stream: Option<stream::DiskStream>,
}

impl MediaBuffer {
    /// Resamples (if needed) and analyses decoded audio. Worker thread only.
    pub fn prepare(decoded: Decoded, rate: u32) -> Result<Self, MediaError> {
        let mut planes = decoded.planes;
        planes.truncate(2);
        let peaks = PeakPyramid::build(&planes);
        let planes = if decoded.info.sample_rate == rate {
            planes
        } else {
            resample(&planes, decoded.info.sample_rate, rate)?
        };
        Ok(Self {
            info: decoded.info,
            rate,
            planes,
            peaks,
            stream: None,
        })
    }

    pub fn frames(&self) -> usize {
        self.stream.as_ref().map_or_else(
            || self.planes.first().map_or(0, Vec::len),
            |s| s.frames() as usize,
        )
    }

    pub fn channels(&self) -> usize {
        usize::from(self.info.channels).min(2)
    }

    /// Engine-rate position of a source frame.
    pub fn engine_frames(&self, source_frames: u64) -> u64 {
        if self.info.sample_rate == self.rate {
            source_frames
        } else {
            elyvon_core::tempo::round_frame(
                source_frames as f64 * f64::from(self.rate) / f64::from(self.info.sample_rate),
            )
        }
    }

    pub fn memory_bytes(&self) -> usize {
        self.planes.iter().map(|p| p.len() * 4).sum::<usize>()
            + self.peaks.memory_bytes()
            + self
                .stream
                .as_ref()
                .map_or(0, |_| stream::DiskStream::cache_bytes())
    }

    /// Nonblocking read for playback; all disk IO belongs to the cache worker.
    #[inline]
    pub fn sample(&self, frame: usize) -> [f32; 2] {
        if let Some(stream) = &self.stream {
            return stream.sample(frame as u64);
        }
        let left = self
            .planes
            .first()
            .and_then(|p| p.get(frame))
            .copied()
            .unwrap_or(0.0);
        let right = self
            .planes
            .last()
            .and_then(|p| p.get(frame))
            .copied()
            .unwrap_or(left);
        [left, right]
    }

    /// [`Self::sample`] for reversed playback (reads ahead backwards).
    #[inline]
    pub fn sample_reverse(&self, frame: usize) -> [f32; 2] {
        match &self.stream {
            Some(stream) => stream.sample_reverse(frame as u64),
            None => self.sample(frame),
        }
    }

    pub fn prefetch(&self, start: u64, frames: usize) -> Result<(), MediaError> {
        self.stream
            .as_ref()
            .map_or(Ok(()), |s| s.prefetch(start, frames))
    }

    pub fn load_streamed(path: &Path, rate: u32, budget: usize) -> Result<Self, MediaError> {
        let (stream, info, peaks) = stream::DiskStream::prepare(path, rate, budget)?;
        let buffer = Self {
            info,
            rate,
            planes: Vec::new(),
            peaks,
            stream: Some(stream),
        };
        if buffer.memory_bytes() > budget {
            return Err(MediaError::TooLarge);
        }
        Ok(buffer)
    }

    /// Min/max per channel of source frames `[start, end)`.
    pub fn peak_range(&self, start: u64, end: u64) -> [(f32, f32); 2] {
        let mut out = [(0.0, 0.0); 2];
        if self.peaks.range(start, end, &mut out) {
            return out;
        }
        let a = self.engine_frames(start) as usize;
        let b = (self.engine_frames(end) as usize)
            .max(a + 1)
            .min(self.frames());
        if a < b {
            if self.prefetch(a as u64, b - a).is_err() {
                return out;
            }
            out = [(f32::MAX, f32::MIN); 2];
            for frame in a..b {
                let pair = self.sample(frame);
                for (ch, slot) in out.iter_mut().enumerate() {
                    slot.0 = slot.0.min(pair[ch]);
                    slot.1 = slot.1.max(pair[ch]);
                }
            }
        }
        out
    }
}

/// Band-limited FFT sample-rate conversion (rubato, MIT/Apache-2.0).
/// Output length is `round(frames × to / from)`.
pub fn resample(planes: &[Vec<f32>], from: u32, to: u32) -> Result<Vec<Vec<f32>>, MediaError> {
    let channels = planes.len();
    let frames = planes.first().map_or(0, Vec::len);
    let expected =
        elyvon_core::tempo::round_frame(frames as f64 * f64::from(to) / f64::from(from)) as usize;
    if frames == 0 {
        return Ok(vec![Vec::new(); channels]);
    }
    let err = |e: &dyn std::fmt::Display| MediaError::Resample(e.to_string());
    let mut resampler =
        Fft::<f32>::new(from as usize, to as usize, 1024, channels, FixedSync::Input)
            .map_err(|e| err(&e))?;
    let input = SequentialSliceOfVecs::new(planes, channels, frames).map_err(|e| err(&e))?;
    let output = resampler
        .process_all(&input, frames, None)
        .map_err(|e| err(&e))?;
    let data = output.take_data();
    let got = data.len() / channels.max(1);
    let mut out = vec![vec![0.0f32; expected]; channels];
    for (frame, chunk) in data
        .chunks_exact(channels)
        .enumerate()
        .take(expected.min(got))
    {
        for (ch, &s) in chunk.iter().enumerate() {
            out[ch][frame] = s;
        }
    }
    Ok(out)
}

/// Load state of one media-pool entry, reported to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum MediaState {
    Loading,
    Ready {
        /// The file differs from the one measured at import.
        changed: bool,
    },
    Failed {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaStatus {
    pub id: MediaId,
    pub state: MediaState,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileKey {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    rate: u32,
}

fn file_key(path: &Path, rate: u32) -> Option<FileKey> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileKey {
        path: path.to_owned(),
        size: meta.len(),
        modified: meta.modified().ok(),
        rate,
    })
}

/// Resolves a pool path: absolute as is, otherwise relative to the project.
pub fn resolve_path(path: &str, project_dir: Option<&Path>) -> PathBuf {
    let p = PathBuf::from(path);
    match project_dir {
        Some(dir) if p.is_relative() => dir.join(p),
        _ => p,
    }
}

/// Default in-memory budget for decoded audio (4 GiB; 512 MiB on 32-bit).
#[cfg(target_pointer_width = "64")]
pub const fn default_budget_bytes() -> usize {
    4 << 30
}

/// Default in-memory budget for decoded audio (512 MiB on 32-bit targets).
#[cfg(not(target_pointer_width = "64"))]
pub const fn default_budget_bytes() -> usize {
    512 << 20
}

/// Decoded media by pool id, plus a content cache so reopening a project or
/// undoing an import does not decode the same file twice.
#[derive(Debug)]
pub struct MediaStore {
    rate: u32,
    budget: usize,
    cache: HashMap<FileKey, Arc<MediaBuffer>>,
    entries: HashMap<MediaId, Result<Arc<MediaBuffer>, MediaError>>,
}

impl MediaStore {
    pub fn new(rate: u32, budget: usize) -> Self {
        Self {
            rate,
            budget,
            cache: HashMap::new(),
            entries: HashMap::new(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Changing the engine rate drops everything prepared for the old rate.
    pub fn set_rate(&mut self, rate: u32) {
        if rate != self.rate {
            self.rate = rate;
            self.cache.clear();
            self.entries.clear();
        }
    }

    pub fn used_bytes(&self) -> usize {
        self.cache.values().map(|b| b.memory_bytes()).sum()
    }

    pub fn remaining_bytes(&self) -> usize {
        self.budget.saturating_sub(self.used_bytes())
    }

    pub fn get(&self, id: MediaId) -> Option<&Result<Arc<MediaBuffer>, MediaError>> {
        self.entries.get(&id)
    }

    pub fn ready(&self, id: MediaId) -> Option<Arc<MediaBuffer>> {
        self.entries.get(&id)?.as_ref().ok().cloned()
    }

    pub fn contains(&self, id: MediaId) -> bool {
        self.entries.contains_key(&id)
    }

    /// Looks up an already prepared file with unchanged size/mtime.
    pub fn cached(&self, path: &Path) -> Option<Arc<MediaBuffer>> {
        self.cache.get(&file_key(path, self.rate)?).cloned()
    }

    pub fn insert(
        &mut self,
        id: MediaId,
        path: &Path,
        result: Result<Arc<MediaBuffer>, MediaError>,
    ) {
        if let (Ok(buffer), Some(key)) = (&result, file_key(path, self.rate))
            && buffer.rate == self.rate
        {
            self.cache.insert(key, Arc::clone(buffer));
        }
        self.entries.insert(id, result);
    }

    /// Forgets pool ids that are no longer in the project (cache kept).
    pub fn retain_ids(&mut self, keep: impl Fn(&MediaId) -> bool) {
        self.entries.retain(|id, _| keep(id));
    }

    /// Status of every asset for the UI.
    pub fn statuses(&self, assets: &[MediaAsset]) -> Vec<MediaStatus> {
        assets
            .iter()
            .map(|a| MediaStatus {
                id: a.id,
                state: match self.entries.get(&a.id) {
                    None => MediaState::Loading,
                    Some(Ok(buf)) if buf.stream.as_ref().is_some_and(|s| s.failed()) => {
                        MediaState::Failed {
                            code: "streamingIo".into(),
                            message: "The disk cache could not be read; playback contains silence."
                                .into(),
                        }
                    }
                    Some(Ok(buf)) => MediaState::Ready {
                        changed: a
                            .info
                            .as_ref()
                            .is_some_and(|i| i.fingerprint != buf.info.fingerprint),
                    },
                    Some(Err(e)) => MediaState::Failed {
                        code: e.code().into(),
                        message: e.to_string(),
                    },
                },
            })
            .collect()
    }

    /// Decodes and prepares one file (blocking; call on a worker thread).
    pub fn load_file(
        path: &Path,
        rate: u32,
        budget: usize,
    ) -> Result<Arc<MediaBuffer>, MediaError> {
        // Small files stay in memory; larger/expanding compressed files use
        // bounded packet decoding and a disk cache, independent of file size.
        let max_frames = (budget.min(16 << 20) / 8) as u64;
        if std::fs::metadata(path).is_ok_and(|m| m.len() > 16 << 20) {
            return MediaBuffer::load_streamed(path, rate, budget).map(Arc::new);
        }
        let decoded = match decode_file(path, max_frames) {
            Ok(decoded) => decoded,
            Err(MediaError::TooLarge) => {
                return MediaBuffer::load_streamed(path, rate, budget).map(Arc::new);
            }
            Err(e) => return Err(e),
        };
        let estimate = (decoded.info.frames as f64 * f64::from(rate)
            / f64::from(decoded.info.sample_rate)) as usize
            * decoded.planes.len().min(2)
            * 4;
        if estimate > budget {
            return MediaBuffer::load_streamed(path, rate, budget).map(Arc::new);
        }
        MediaBuffer::prepare(decoded, rate).map(Arc::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin() as f32 * 0.5
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn resampling_preserves_level_frequency_and_length() {
        for (from, to) in [
            (44_100, 48_000),
            (48_000, 44_100),
            (96_000, 48_000),
            (22_050, 48_000),
        ] {
            let input = sine(1000.0, from, from as usize); // 1 s
            let out = resample(&[input.clone(), input], from, to).unwrap();
            assert_eq!(out[0].len(), to as usize, "{from}->{to}");
            // Ignore filter edges; compare the middle against an ideal sine.
            let mid = &out[0][(to / 4) as usize..(3 * to / 4) as usize];
            assert!((rms(mid) - 0.5 / 2f64.sqrt()).abs() < 0.005, "{from}->{to}");
            let ideal = sine(1000.0, to, to as usize);
            let err: f64 = mid
                .iter()
                .zip(&ideal[(to / 4) as usize..])
                .map(|(a, b)| f64::from(a - b).powi(2))
                .sum::<f64>()
                / mid.len() as f64;
            assert!(
                err.sqrt() < 0.01,
                "phase/shape error {from}->{to}: {}",
                err.sqrt()
            );
        }
    }

    #[test]
    fn store_reports_states() {
        let mut store = MediaStore::new(48_000, 1 << 20);
        let id = MediaId::new();
        let asset = MediaAsset {
            id,
            name: "x.wav".into(),
            path: "/nope/x.wav".into(),
            info: None,
        };
        assert_eq!(
            store.statuses(std::slice::from_ref(&asset))[0].state,
            MediaState::Loading
        );
        let err = MediaStore::load_file(Path::new("/nope/x.wav"), 48_000, 1 << 20).unwrap_err();
        assert_eq!(err.code(), "missing");
        store.insert(id, Path::new("/nope/x.wav"), Err(err));
        assert!(matches!(
            store.statuses(&[asset])[0].state,
            MediaState::Failed { ref code, .. } if code == "missing"
        ));
    }
}
