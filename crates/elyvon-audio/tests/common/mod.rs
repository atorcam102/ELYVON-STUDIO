//! Shared helpers for engine integration tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use elyvon_audio::contracts::{MidiEvent, MidiMessage};
use elyvon_audio::engine::MidiSink;
use elyvon_audio::media::{Decoded, MediaBuffer};
use elyvon_core::Ticks;
use elyvon_project::{
    AudioPlacement, EditCommand, MediaAsset, MediaId, MediaInfo, PlacementTrack, Project, RegionId,
    TrackId, TrackKind,
};

pub fn sine(freq: f64, amp: f64, rate: u32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin()) as f32
        })
        .collect()
}

pub fn info(rate: u32, channels: u16, frames: u64) -> MediaInfo {
    MediaInfo {
        sample_rate: rate,
        channels,
        frames,
        format: "test".into(),
        size_bytes: 0,
        fingerprint: "0".into(),
    }
}

/// A prepared buffer from synthetic planes (bypasses decoding).
pub fn buffer(planes: Vec<Vec<f32>>, source_rate: u32, engine_rate: u32) -> Arc<MediaBuffer> {
    let frames = planes[0].len() as u64;
    let channels = planes.len() as u16;
    Arc::new(
        MediaBuffer::prepare(
            Decoded {
                info: info(source_rate, channels, frames),
                planes,
            },
            engine_rate,
        )
        .unwrap(),
    )
}

/// A project plus the buffers its media ids resolve to.
pub struct Session {
    pub project: Project,
    pub media: HashMap<MediaId, Arc<MediaBuffer>>,
}

impl Session {
    pub fn new(bpm: f64) -> Self {
        let mut project = Project::new("Test");
        project.transport.tempo = elyvon_core::Tempo::new(bpm).unwrap();
        Self {
            project,
            media: HashMap::new(),
        }
    }

    pub fn lookup(&self) -> impl Fn(MediaId) -> Option<Arc<MediaBuffer>> + '_ {
        move |id| self.media.get(&id).cloned()
    }

    pub fn apply(&mut self, cmd: EditCommand) -> elyvon_project::EditOutcome {
        elyvon_project::edit::apply(&mut self.project, &cmd).unwrap()
    }

    pub fn audio_track(&mut self) -> TrackId {
        self.apply(EditCommand::AddTrack {
            kind: TrackKind::Audio,
            name: None,
            index: None,
        })
        .created_tracks[0]
    }

    /// Adds `buffer` to the pool and places it on `track` at `start`.
    pub fn place(
        &mut self,
        track: TrackId,
        start: Ticks,
        buf: &Arc<MediaBuffer>,
        name: &str,
    ) -> RegionId {
        let id = MediaId::new();
        self.media.insert(id, Arc::clone(buf));
        self.apply(EditCommand::ImportAudio {
            assets: vec![MediaAsset {
                id,
                name: name.into(),
                path: format!("/virtual/{name}"),
                info: Some(buf.info.clone()),
            }],
            placements: vec![AudioPlacement {
                media_id: id,
                track: PlacementTrack::Existing { id: track },
                start,
            }],
        })
        .created_regions[0]
    }
}

/// Records every MIDI event with its absolute engine frame.
#[derive(Debug, Default)]
pub struct MidiCapture {
    pub block_start: u64,
    pub events: Vec<(u64, usize, MidiMessage)>,
}

impl MidiSink for MidiCapture {
    fn event(&mut self, track: usize, event: MidiEvent) {
        self.events.push((
            self.block_start + u64::from(event.frame_offset),
            track,
            event.message,
        ));
    }
}

fn wav_header(out: &mut Vec<u8>, format: u16, channels: u16, rate: u32, bits: u16, data_len: u32) {
    let block = channels * bits / 8;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
}

/// Writes planar samples as PCM 16/24-bit or float-32 WAV.
pub fn write_wav(path: &Path, planes: &[Vec<f32>], rate: u32, bits: u16) {
    let channels = planes.len() as u16;
    let frames = planes[0].len();
    let mut data = Vec::new();
    for i in 0..frames {
        for p in planes {
            let s = p[i];
            match bits {
                16 => data.extend_from_slice(&((s * 32767.0).round() as i16).to_le_bytes()),
                24 => {
                    let v = (s * 8_388_607.0).round() as i32;
                    data.extend_from_slice(&v.to_le_bytes()[..3]);
                }
                32 => data.extend_from_slice(&s.to_le_bytes()),
                _ => panic!("bits"),
            }
        }
    }
    let mut out = Vec::new();
    wav_header(
        &mut out,
        if bits == 32 { 3 } else { 1 },
        channels,
        rate,
        bits,
        data.len() as u32,
    );
    out.extend_from_slice(&data);
    std::fs::write(path, out).unwrap();
}

/// 80-bit IEEE extended for the AIFF COMM sample rate.
fn extended(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    let v = rate as u64;
    let shift = v.leading_zeros();
    let exponent = 16383 + 63 - shift as u16;
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&(v << shift).to_be_bytes());
    out
}

/// Writes 16-bit big-endian AIFF.
pub fn write_aiff(path: &Path, planes: &[Vec<f32>], rate: u32) {
    let channels = planes.len() as u16;
    let frames = planes[0].len() as u32;
    let mut sound = Vec::new();
    for i in 0..frames as usize {
        for p in planes {
            sound.extend_from_slice(&((p[i] * 32767.0).round() as i16).to_be_bytes());
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&(4 + 26 + 16 + sound.len() as u32).to_be_bytes());
    out.extend_from_slice(b"AIFFCOMM");
    out.extend_from_slice(&18u32.to_be_bytes());
    out.extend_from_slice(&channels.to_be_bytes());
    out.extend_from_slice(&frames.to_be_bytes());
    out.extend_from_slice(&16u16.to_be_bytes());
    out.extend_from_slice(&extended(rate));
    out.extend_from_slice(b"SSND");
    out.extend_from_slice(&(8 + sound.len() as u32).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&sound);
    std::fs::write(path, out).unwrap();
}

pub fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

pub fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
