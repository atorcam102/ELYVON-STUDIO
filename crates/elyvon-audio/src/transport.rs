//! Transport commands (control → audio thread) and the state the audio thread
//! publishes back through atomics (audio thread → UI). Nothing here locks.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::plan::MAX_TRACKS;
use crate::rt::AtomicF32;

/// A transport request, applied at the start of the next audio block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TransportCommand {
    /// Starts from `from_tick` (or the current position). Pre-roll starts the
    /// given number of bars earlier; count-in clicks first without playing.
    Play {
        #[ts(optional, type = "number")]
        from_tick: Option<u64>,
        count_in: bool,
        pre_roll: bool,
    },
    /// Stops and keeps the position. Stopping while stopped returns to where
    /// playback last started.
    Stop,
    /// Stops and keeps the position (resume with Play).
    Pause,
    /// Moves the playhead, while playing or not.
    Locate {
        #[ts(type = "number")]
        tick: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[repr(u8)]
pub enum PlayState {
    Stopped = 0,
    Paused = 1,
    CountIn = 2,
    Playing = 3,
}

impl PlayState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Paused,
            2 => Self::CountIn,
            3 => Self::Playing,
            _ => Self::Stopped,
        }
    }
}

/// State published by the audio thread after every block.
#[derive(Debug)]
pub struct EngineShared {
    pub sample_rate: u32,
    state: AtomicU8,
    position: AtomicU64,
    position_ticks: AtomicU64,
    count_in_remaining: AtomicU64,
    plan_generation: AtomicU64,
    cycles: AtomicU64,
    blocks: AtomicU64,
    frames: AtomicU64,
    xruns: AtomicU64,
    last_block_ns: AtomicU64,
    max_block_ns: AtomicU64,
    midi_overflow: AtomicU64,
    master_peak: [AtomicF32; 2],
    track_peaks: Vec<AtomicF32>,
}

impl EngineShared {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            state: AtomicU8::new(0),
            position: AtomicU64::new(0),
            position_ticks: AtomicU64::new(0f64.to_bits()),
            count_in_remaining: AtomicU64::new(0),
            plan_generation: AtomicU64::new(0),
            cycles: AtomicU64::new(0),
            blocks: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            xruns: AtomicU64::new(0),
            last_block_ns: AtomicU64::new(0),
            max_block_ns: AtomicU64::new(0),
            midi_overflow: AtomicU64::new(0),
            master_peak: [AtomicF32::new(0.0), AtomicF32::new(0.0)],
            track_peaks: (0..MAX_TRACKS).map(|_| AtomicF32::new(0.0)).collect(),
        }
    }

    // ---- audio-thread side (stores only) ----

    pub(crate) fn publish(
        &self,
        state: PlayState,
        position: u64,
        ticks: f64,
        count_in: u64,
        generation: u64,
    ) {
        self.position.store(position, Ordering::Relaxed);
        self.position_ticks
            .store(ticks.to_bits(), Ordering::Relaxed);
        self.count_in_remaining.store(count_in, Ordering::Relaxed);
        self.plan_generation.store(generation, Ordering::Relaxed);
        self.state.store(state as u8, Ordering::Release);
    }

    pub(crate) fn add_block(&self, frames: u64, nanos: u64) {
        self.blocks.fetch_add(1, Ordering::Relaxed);
        self.frames.fetch_add(frames, Ordering::Relaxed);
        self.last_block_ns.store(nanos, Ordering::Relaxed);
        self.max_block_ns.fetch_max(nanos, Ordering::Relaxed);
    }

    pub(crate) fn add_cycle(&self) {
        self.cycles.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn add_midi_overflow(&self) {
        self.midi_overflow.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn peak_master(&self, ch: usize, v: f32) {
        let slot = &self.master_peak[ch];
        if v > slot.load() {
            slot.store(v);
        }
    }

    #[inline]
    pub(crate) fn peak_track(&self, i: usize, v: f32) {
        if let Some(slot) = self.track_peaks.get(i)
            && v > slot.load()
        {
            slot.store(v);
        }
    }

    // ---- control side ----

    /// Device-reported buffer under/overruns (from the stream error callback).
    pub fn add_xrun(&self) {
        self.xruns.fetch_add(1, Ordering::Relaxed);
    }

    pub fn state(&self) -> PlayState {
        PlayState::from_u8(self.state.load(Ordering::Acquire))
    }

    pub fn snapshot(&self) -> TransportSnapshot {
        let state = self.state();
        let samples = self.position.load(Ordering::Relaxed);
        TransportSnapshot {
            state,
            samples,
            ticks: f64::from_bits(self.position_ticks.load(Ordering::Relaxed)),
            seconds: samples as f64 / f64::from(self.sample_rate),
            count_in_remaining: self.count_in_remaining.load(Ordering::Relaxed) as f64
                / f64::from(self.sample_rate),
            plan_generation: self.plan_generation.load(Ordering::Relaxed),
            cycles: self.cycles.load(Ordering::Relaxed),
        }
    }

    pub fn stats(&self) -> EngineStats {
        EngineStats {
            blocks: self.blocks.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            xruns: self.xruns.load(Ordering::Relaxed),
            last_block_us: self.last_block_ns.load(Ordering::Relaxed) as f64 / 1000.0,
            max_block_us: self.max_block_ns.load(Ordering::Relaxed) as f64 / 1000.0,
            midi_overflow: self.midi_overflow.load(Ordering::Relaxed),
        }
    }

    /// Reads and resets peak levels (master L/R, then `tracks` tracks).
    pub fn take_peaks(&self, tracks: usize) -> Meters {
        let take = |a: &AtomicF32| {
            let v = a.load();
            a.store(0.0);
            v
        };
        Meters {
            master: [take(&self.master_peak[0]), take(&self.master_peak[1])],
            tracks: self.track_peaks.iter().take(tracks).map(take).collect(),
        }
    }
}

/// Playhead and transport state as observed from the audio thread.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TransportSnapshot {
    pub state: PlayState,
    /// Engine frame of the playhead.
    #[ts(type = "number")]
    pub samples: u64,
    /// Musical position (fractional ticks) through the engine's tempo map.
    pub ticks: f64,
    pub seconds: f64,
    /// Seconds of count-in left (0 when not counting in).
    pub count_in_remaining: f64,
    #[ts(type = "number")]
    pub plan_generation: u64,
    /// Cycle jumps since the engine started.
    #[ts(type = "number")]
    pub cycles: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EngineStats {
    #[ts(type = "number")]
    pub blocks: u64,
    #[ts(type = "number")]
    pub frames: u64,
    #[ts(type = "number")]
    pub xruns: u64,
    pub last_block_us: f64,
    pub max_block_us: f64,
    #[ts(type = "number")]
    pub midi_overflow: u64,
}

/// Peak levels (linear) since the previous read.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Meters {
    pub master: [f32; 2],
    pub tracks: Vec<f32>,
}
