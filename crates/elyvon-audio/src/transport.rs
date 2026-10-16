//! Transport commands (control → audio thread) and the state the audio thread
//! publishes back through atomics (audio thread → UI). Nothing here locks.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use std::sync::atomic::{AtomicBool, AtomicU32};

use crate::plan::{MASTER_SLOT, MAX_CHANNELS, aux_slot};

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
    meters: Vec<MeterSlot>,
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
            meters: (0..MAX_CHANNELS).map(|_| MeterSlot::default()).collect(),
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

    /// Audio thread: accumulates one block of a strip's post-fader signal.
    #[inline]
    pub(crate) fn meter(
        &self,
        slot: usize,
        peak: [f32; 2],
        sum: [f64; 2],
        frames: u64,
        clip: bool,
    ) {
        if let Some(m) = self.meters.get(slot) {
            m.add(peak, sum, frames, clip);
        }
    }

    /// Clears every clip indicator (control side).
    pub fn reset_clips(&self) {
        for m in &self.meters {
            m.clip.store(false, Ordering::Relaxed);
        }
    }

    /// Reads levels since the previous call: peaks, a 300 ms RMS and sticky
    /// clip flags of the master, the first `tracks` tracks and `auxes` auxes.
    pub fn read_meters(&self, reader: &mut MeterReader, tracks: usize, auxes: usize) -> Meters {
        let mut read = |slot: usize| -> ChannelMeter {
            let m = &self.meters[slot];
            let r = &mut reader.slots[slot];
            let frames = m.frames.load(Ordering::Acquire);
            let mut out = ChannelMeter::default();
            let df = frames.wrapping_sub(r.frames);
            for ch in 0..2 {
                out.peak[ch] = f32::from_bits(m.peak[ch].swap(0, Ordering::AcqRel));
                let sum = f64::from_bits(m.sum[ch].load(Ordering::Acquire));
                let block = if df > 0 {
                    (sum - r.sum[ch]).max(0.0) / df as f64
                } else {
                    0.0
                };
                r.sum[ch] = sum;
                // Exponential average, ~300 ms at the ~30 Hz read rate.
                r.ms[ch] += (block - r.ms[ch]) * 0.104;
                out.rms[ch] = r.ms[ch].sqrt() as f32;
            }
            r.frames = frames;
            out.clip = m.clip.load(Ordering::Relaxed);
            out
        };
        Meters {
            master: read(MASTER_SLOT),
            tracks: (0..tracks.min(crate::plan::MAX_TRACKS))
                .map(&mut read)
                .collect(),
            auxes: (0..auxes.min(crate::plan::MAX_AUXES))
                .map(|a| read(aux_slot(a)))
                .collect(),
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
}

/// Lock-free accumulator written only by the audio thread. Totals grow
/// monotonically so the reader can take differences without a reset race;
/// peaks are swapped out by the reader (`fetch_max` on non-negative f32 bits).
#[derive(Debug, Default)]
pub struct MeterSlot {
    peak: [AtomicU32; 2],
    sum: [AtomicU64; 2],
    frames: AtomicU64,
    clip: AtomicBool,
}

impl MeterSlot {
    #[inline]
    fn add(&self, peak: [f32; 2], sum: [f64; 2], frames: u64, clip: bool) {
        for ch in 0..2 {
            self.peak[ch].fetch_max(peak[ch].max(0.0).to_bits(), Ordering::AcqRel);
            let total = f64::from_bits(self.sum[ch].load(Ordering::Relaxed)) + sum[ch];
            self.sum[ch].store(total.to_bits(), Ordering::Release);
        }
        self.frames.fetch_add(frames, Ordering::Release);
        if clip {
            self.clip.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ReaderSlot {
    sum: [f64; 2],
    frames: u64,
    ms: [f64; 2],
}

/// Control-side state of [`EngineShared::read_meters`].
#[derive(Debug, Clone)]
pub struct MeterReader {
    slots: Vec<ReaderSlot>,
}

impl Default for MeterReader {
    fn default() -> Self {
        Self {
            slots: vec![ReaderSlot::default(); MAX_CHANNELS],
        }
    }
}

/// Levels of one channel strip (linear, post fader).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChannelMeter {
    /// Highest sample since the previous read, L/R.
    pub peak: [f32; 2],
    /// ~300 ms RMS, L/R.
    pub rms: [f32; 2],
    /// A sample exceeded full scale (or was not finite) since the last reset.
    pub clip: bool,
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

/// Meter levels of the master, tracks (project order) and auxiliary channels
/// (mixer order) measured on the real signal by the audio thread.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Meters {
    pub master: ChannelMeter,
    pub tracks: Vec<ChannelMeter>,
    pub auxes: Vec<ChannelMeter>,
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
