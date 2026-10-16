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

const CLOCK_HISTORY: usize = 2048;
#[derive(Debug, Default)]
struct ClockSpan {
    version: AtomicU64,
    start: AtomicU64,
    end: AtomicU64,
    song: AtomicU64,
    state: AtomicU64,
    cycle: AtomicU64,
}
/// State published by the audio thread after every block.
#[derive(Debug)]
pub struct EngineShared {
    clock_history: Vec<ClockSpan>,
    clock_cursor: AtomicU64,
    epoch: std::time::Instant,
    clock_ns: AtomicU64,
    clock_version: AtomicU64,
    block_frames: AtomicU64,
    output_latency: AtomicU64,
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
    streaming_underruns: AtomicU64,
    master_peak: [AtomicF32; 2],
    track_peaks: Vec<AtomicF32>,
}

impl EngineShared {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            clock_history: (0..CLOCK_HISTORY).map(|_| ClockSpan::default()).collect(),
            clock_cursor: AtomicU64::new(0),
            epoch: std::time::Instant::now(),
            clock_ns: AtomicU64::new(0),
            clock_version: AtomicU64::new(0),
            block_frames: AtomicU64::new(0),
            output_latency: AtomicU64::new(0),
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
            streaming_underruns: AtomicU64::new(0),
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
        frames: u64,
    ) {
        self.clock_version.fetch_add(1, Ordering::SeqCst);
        self.clock_ns
            .store(self.epoch.elapsed().as_nanos() as u64, Ordering::SeqCst);
        self.block_frames.store(frames, Ordering::SeqCst);
        self.position.store(position, Ordering::Relaxed);
        self.position_ticks
            .store(ticks.to_bits(), Ordering::Relaxed);
        self.count_in_remaining.store(count_in, Ordering::Relaxed);
        self.plan_generation.store(generation, Ordering::Relaxed);
        self.state.store(state as u8, Ordering::Release);
        self.clock_version.fetch_add(1, Ordering::SeqCst);
    }

    pub fn set_output_latency(&self, frames: u64) {
        self.output_latency.store(frames, Ordering::Relaxed);
    }
    pub fn output_latency(&self) -> u64 {
        self.output_latency.load(Ordering::Relaxed)
    }

    pub(crate) fn clock_now(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }
    /// Publish each real transport span, including count-in/cycle boundaries.
    /// Input/MIDI timestamps can then be mapped into an older callback rather
    /// than extrapolated through a discontinuity. Fixed storage, no RT locks.
    pub(crate) fn record_clock(
        &self,
        callback_ns: u64,
        offset: u32,
        frames: usize,
        song: u64,
        state: PlayState,
    ) {
        if frames == 0 {
            return;
        }
        let cursor = self.clock_cursor.load(Ordering::Relaxed);
        let slot = &self.clock_history[cursor as usize % CLOCK_HISTORY];
        slot.version.fetch_add(1, Ordering::SeqCst);
        slot.start.store(
            callback_ns + u64::from(offset) * 1_000_000_000 / u64::from(self.sample_rate),
            Ordering::SeqCst,
        );
        slot.end.store(
            callback_ns
                + (u64::from(offset) + frames as u64) * 1_000_000_000 / u64::from(self.sample_rate),
            Ordering::SeqCst,
        );
        slot.song.store(song, Ordering::SeqCst);
        slot.state.store(state as u64, Ordering::SeqCst);
        slot.cycle
            .store(self.cycles.load(Ordering::Relaxed), Ordering::SeqCst);
        slot.version.fetch_add(1, Ordering::SeqCst);
        self.clock_cursor
            .store(cursor.wrapping_add(1), Ordering::Release);
    }
    fn capture_clock(&self, when: u64) -> Option<(u64, PlayState, u64)> {
        let cursor = self.clock_cursor.load(Ordering::Acquire);
        for back in 0..cursor.min(CLOCK_HISTORY as u64) {
            let slot = &self.clock_history[(cursor - 1 - back) as usize % CLOCK_HISTORY];
            let version = slot.version.load(Ordering::SeqCst);
            if version & 1 != 0 {
                continue;
            }
            let start = slot.start.load(Ordering::SeqCst);
            let end = slot.end.load(Ordering::SeqCst);
            if when < start || when >= end {
                continue;
            }
            let song = slot.song.load(Ordering::SeqCst);
            let state = PlayState::from_u8(slot.state.load(Ordering::SeqCst) as u8);
            let cycle = slot.cycle.load(Ordering::SeqCst);
            if slot.version.load(Ordering::SeqCst) != version {
                continue;
            }
            let advance = if state == PlayState::Playing {
                (when - start) * u64::from(self.sample_rate) / 1_000_000_000
            } else {
                0
            };
            return Some((song.saturating_add(advance), state, cycle));
        }
        None
    }

    /// Bounded snapshot of the output clock, mapped to an input capture time.
    /// Separate device clocks make this an estimate, not a loopback calibration.
    pub fn capture_position(&self, age: std::time::Duration) -> Option<(u64, PlayState, u64)> {
        let when = self
            .clock_now()
            .saturating_sub(age.as_nanos().min(u128::from(u64::MAX)) as u64);
        if let Some(position) = self.capture_clock(when) {
            return Some(position);
        }

        for _ in 0..3 {
            let v = self.clock_version.load(Ordering::SeqCst);
            if v & 1 != 0 {
                continue;
            }
            let clock = self.clock_ns.load(Ordering::SeqCst);
            let samples = self.position.load(Ordering::Relaxed);
            let frames = self.block_frames.load(Ordering::SeqCst);
            let state = self.state();
            let cycles = self.cycles.load(Ordering::Relaxed);
            if self.clock_version.load(Ordering::SeqCst) != v {
                continue;
            }
            let elapsed = self
                .epoch
                .elapsed()
                .as_nanos()
                .saturating_sub(u128::from(clock));
            let advance = elapsed.saturating_mul(u128::from(self.sample_rate)) / 1_000_000_000;
            let input = (age.as_nanos() * u128::from(self.sample_rate) / 1_000_000_000) as u64;
            let position = samples
                .saturating_sub(frames)
                .saturating_add(advance as u64)
                .saturating_sub(self.output_latency())
                .saturating_sub(input);
            return Some((position, state, cycles));
        }
        None
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

    pub(crate) fn add_streaming_underruns(&self, n: u64) {
        self.streaming_underruns.fetch_add(n, Ordering::Relaxed);
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
            streaming_underruns: self.streaming_underruns.load(Ordering::Relaxed),
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
    #[ts(type = "number")]
    pub streaming_underruns: u64,
}

/// Peak levels (linear) since the previous read.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Meters {
    pub master: [f32; 2],
    pub tracks: Vec<f32>,
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    #[test]
    fn capture_timestamps_map_through_count_in_and_cycle_boundaries() {
        let clock = EngineShared::new(48000);
        clock.record_clock(1_000_000_000, 0, 24000, 0, PlayState::CountIn);
        clock.record_clock(1_500_000_000, 0, 4800, 0, PlayState::Playing);
        clock.add_cycle();
        clock.record_clock(1_600_000_000, 0, 4800, 0, PlayState::Playing);
        assert_eq!(
            clock.capture_clock(1_400_000_000),
            Some((0, PlayState::CountIn, 0))
        );
        assert_eq!(
            clock.capture_clock(1_550_000_000),
            Some((2400, PlayState::Playing, 0))
        );
        assert_eq!(
            clock.capture_clock(1_600_000_000),
            Some((0, PlayState::Playing, 1))
        );
        assert_eq!(
            clock.capture_clock(1_650_000_000),
            Some((2400, PlayState::Playing, 1))
        );
    }
}
