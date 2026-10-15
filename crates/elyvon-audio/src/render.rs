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
    while done < frames {
        let n = block.min(frames - done);
        let chunk = &mut buf[..n * 2];
        engine.process(chunk, 2, midi);
        for frame in chunk.chunks_exact(2) {
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
    let wav = encode_wav_f32(&rendered.samples, 2, rendered.report.sample_rate);
    elyvon_core::fs::atomic_write(path, &wav)?;
    let mut report_path = path.as_os_str().to_owned();
    report_path.push(".json");
    let report_path = std::path::PathBuf::from(report_path);
    let json = serde_json::to_vec_pretty(&rendered.report).map_err(std::io::Error::other)?;
    elyvon_core::fs::atomic_write(&report_path, &json)?;
    Ok(report_path)
}
