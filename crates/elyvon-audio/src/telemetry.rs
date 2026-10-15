//! Real-time → control telemetry: meters, clocks and stream health.
//!
//! The audio thread only performs relaxed atomic stores/`fetch_max`/`fetch_add`
//! on a pre-allocated [`Telemetry`] block. A control thread periodically takes
//! a [`TelemetrySnapshot`] (which allocates, so it never runs in the callback),
//! logs anomalies and forwards values to the UI. Meter peaks are *held* until
//! read, so a short transient is never lost between two UI frames.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering::Relaxed};

use serde::Serialize;
use ts_rs::TS;

use crate::gain::{SILENCE_DB, gain_to_db};

/// Channels metered per direction. Larger devices are metered on the first 32.
pub const MAX_METER_CHANNELS: usize = 32;

/// Why a running stream can no longer be trusted. Set by the backend's error
/// callback (allocation-free), consumed by the control thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StreamFault {
    /// The device disappeared (unplugged, driver reset, Bluetooth dropped).
    DeviceLost,
    /// The OS changed the device format or route; the stream must be rebuilt.
    Invalidated,
    /// Any other unrecoverable backend error.
    Fatal,
}

impl StreamFault {
    fn to_u8(self) -> u8 {
        match self {
            Self::DeviceLost => 1,
            Self::Invalidated => 2,
            Self::Fatal => 3,
        }
    }

    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::DeviceLost),
            2 => Some(Self::Invalidated),
            3 => Some(Self::Fatal),
            _ => None,
        }
    }
}

/// Shared lock-free telemetry block. Construct on the control thread.
#[derive(Debug)]
pub struct Telemetry {
    clock_frames: AtomicU64,
    transport_frames: AtomicU64,
    playing: AtomicBool,
    tone_active: AtomicBool,
    sample_rate: AtomicU32,
    output_channels: AtomicU32,
    input_channels: AtomicU32,
    output_peaks: [AtomicU32; MAX_METER_CHANNELS],
    input_peaks: [AtomicU32; MAX_METER_CHANNELS],
    clipped_samples: AtomicU64,
    callbacks: AtomicU64,
    input_callbacks: AtomicU64,
    late_callbacks: AtomicU64,
    timing_gaps: AtomicU64,
    backend_xruns: AtomicU64,
    stream_errors: AtomicU64,
    max_load_permille: AtomicU32,
    last_block_frames: AtomicU32,
    max_block_frames: AtomicU32,
    output_latency_us: AtomicU64,
    realtime_denied: AtomicBool,
    input_lost: AtomicBool,
    fault: AtomicU8,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            clock_frames: AtomicU64::new(0),
            transport_frames: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            tone_active: AtomicBool::new(false),
            sample_rate: AtomicU32::new(0),
            output_channels: AtomicU32::new(0),
            input_channels: AtomicU32::new(0),
            output_peaks: std::array::from_fn(|_| AtomicU32::new(0)),
            input_peaks: std::array::from_fn(|_| AtomicU32::new(0)),
            clipped_samples: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            input_callbacks: AtomicU64::new(0),
            late_callbacks: AtomicU64::new(0),
            timing_gaps: AtomicU64::new(0),
            backend_xruns: AtomicU64::new(0),
            stream_errors: AtomicU64::new(0),
            max_load_permille: AtomicU32::new(0),
            last_block_frames: AtomicU32::new(0),
            max_block_frames: AtomicU32::new(0),
            output_latency_us: AtomicU64::new(0),
            realtime_denied: AtomicBool::new(false),
            input_lost: AtomicBool::new(false),
            fault: AtomicU8::new(0),
        }
    }

    // ---- written by the real-time thread (all wait-free) ----

    #[inline]
    pub(crate) fn publish_clock(&self, clock: u64, transport: u64, playing: bool, tone: bool) {
        self.clock_frames.store(clock, Relaxed);
        self.transport_frames.store(transport, Relaxed);
        self.playing.store(playing, Relaxed);
        self.tone_active.store(tone, Relaxed);
    }

    /// Holds the maximum absolute sample value of an output channel.
    #[inline]
    pub fn hold_output_peak(&self, channel: usize, peak: f32) {
        if let Some(slot) = self.output_peaks.get(channel) {
            // Non-negative IEEE-754 floats sort like their bit patterns.
            slot.fetch_max(peak.abs().to_bits(), Relaxed);
        }
    }

    #[inline]
    pub fn hold_input_peak(&self, channel: usize, peak: f32) {
        if let Some(slot) = self.input_peaks.get(channel)
            && peak.is_finite()
        {
            slot.fetch_max(peak.abs().to_bits(), Relaxed);
        }
    }

    #[inline]
    pub fn add_clipped(&self, samples: u64) {
        if samples > 0 {
            self.clipped_samples.fetch_add(samples, Relaxed);
        }
    }

    #[inline]
    pub fn count_input_callback(&self) {
        self.input_callbacks.fetch_add(1, Relaxed);
    }

    /// Backend-reported underrun/overrun (e.g. ALSA EPIPE). Error-callback safe.
    #[inline]
    pub fn count_backend_xrun(&self) {
        self.backend_xruns.fetch_add(1, Relaxed);
    }

    /// Non-fatal backend error reported through the error callback.
    #[inline]
    pub fn count_stream_error(&self) {
        self.stream_errors.fetch_add(1, Relaxed);
    }

    /// The OS refused real-time scheduling for the audio thread.
    #[inline]
    pub fn flag_realtime_denied(&self) {
        self.realtime_denied.store(true, Relaxed);
    }

    /// The metering input disappeared; the output keeps running.
    #[inline]
    pub fn set_input_lost(&self) {
        self.input_lost.store(true, Relaxed);
    }

    /// Marks the stream as unusable. The first fault wins.
    #[inline]
    pub fn set_fault(&self, fault: StreamFault) {
        let _ = self
            .fault
            .compare_exchange(0, fault.to_u8(), Relaxed, Relaxed);
    }

    // ---- control thread ----

    /// Configuration of the stream feeding this block (control thread).
    pub fn set_stream_format(&self, sample_rate: u32, output_channels: u16, input_channels: u16) {
        self.sample_rate.store(sample_rate, Relaxed);
        self.output_channels
            .store(u32::from(output_channels), Relaxed);
        self.input_channels
            .store(u32::from(input_channels), Relaxed);
    }

    pub fn set_input_channels(&self, channels: u16) {
        self.input_channels.store(u32::from(channels), Relaxed);
    }

    /// Takes and clears the pending fault, if any.
    pub fn take_fault(&self) -> Option<StreamFault> {
        StreamFault::from_u8(self.fault.swap(0, Relaxed))
    }

    /// Takes and clears the input-lost flag.
    pub fn take_input_lost(&self) -> bool {
        self.input_lost.swap(false, Relaxed)
    }

    pub fn tone_active(&self) -> bool {
        self.tone_active.load(Relaxed)
    }

    pub fn clock_frames(&self) -> u64 {
        self.clock_frames.load(Relaxed)
    }

    pub fn transport_frames(&self) -> u64 {
        self.transport_frames.load(Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Relaxed)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Relaxed)
    }

    /// Resets the per-stream health counters (not the monotonic clock).
    pub fn reset_stream_stats(&self) {
        for a in [
            &self.clipped_samples,
            &self.callbacks,
            &self.input_callbacks,
            &self.late_callbacks,
            &self.timing_gaps,
            &self.backend_xruns,
            &self.stream_errors,
            &self.output_latency_us,
        ] {
            a.store(0, Relaxed);
        }
        self.max_load_permille.store(0, Relaxed);
        self.realtime_denied.store(false, Relaxed);
        self.input_lost.store(false, Relaxed);
        self.input_channels.store(0, Relaxed);
        self.last_block_frames.store(0, Relaxed);
        self.max_block_frames.store(0, Relaxed);
        for p in self.output_peaks.iter().chain(&self.input_peaks) {
            p.store(0, Relaxed);
        }
        self.fault.store(0, Relaxed);
    }

    /// Reads the health counters without touching the held meter peaks.
    pub fn stats(&self) -> StreamStats {
        let latency_us = self.output_latency_us.load(Relaxed);
        StreamStats {
            callbacks: self.callbacks.load(Relaxed),
            input_callbacks: self.input_callbacks.load(Relaxed),
            late_callbacks: self.late_callbacks.load(Relaxed),
            timing_gaps: self.timing_gaps.load(Relaxed),
            backend_xruns: self.backend_xruns.load(Relaxed),
            stream_errors: self.stream_errors.load(Relaxed),
            clipped_samples: self.clipped_samples.load(Relaxed),
            max_callback_load: self.max_load_permille.load(Relaxed) as f32 / 1000.0,
            last_block_frames: self.last_block_frames.load(Relaxed),
            max_block_frames: self.max_block_frames.load(Relaxed),
            measured_output_latency_ms: (latency_us > 0).then(|| latency_us as f32 / 1000.0),
            realtime_denied: self.realtime_denied.load(Relaxed),
        }
    }

    /// Reads everything and clears the held peaks. Allocates: never call from
    /// the audio callback.
    pub fn snapshot(&self) -> TelemetrySnapshot {
        let take_peaks = |peaks: &[AtomicU32], n: u32| -> Vec<f32> {
            peaks
                .iter()
                .take(n as usize)
                .map(|p| peak_db(f32::from_bits(p.swap(0, Relaxed))))
                .collect()
        };
        let sample_rate = self.sample_rate.load(Relaxed);
        TelemetrySnapshot {
            clock_frames: self.clock_frames.load(Relaxed),
            transport_frames: self.transport_frames.load(Relaxed),
            position_ticks: None,
            playing: self.playing.load(Relaxed),
            tone_active: self.tone_active.load(Relaxed),
            sample_rate,
            output_peak_db: take_peaks(&self.output_peaks, self.output_channels.load(Relaxed)),
            input_peak_db: take_peaks(&self.input_peaks, self.input_channels.load(Relaxed)),
            stats: self.stats(),
        }
    }
}

fn peak_db(peak: f32) -> f32 {
    if peak.is_finite() && peak > 0.0 {
        gain_to_db(f64::from(peak)) as f32
    } else {
        SILENCE_DB as f32
    }
}

/// Per-stream timing monitor. Lives inside the callback closure (plain fields,
/// no atomics) and publishes counters into [`Telemetry`].
#[derive(Debug, Clone)]
pub struct CallbackMonitor {
    sample_rate: u32,
    last_callback_ns: Option<u64>,
    last_frames: u64,
}

impl CallbackMonitor {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            last_callback_ns: None,
            last_frames: 0,
        }
    }

    /// Records one output callback. `callback_ns` is the backend's callback
    /// timestamp, `elapsed_ns` how long our processing took and `latency_ns`
    /// the backend's predicted callback → playback delay. Wait-free.
    #[inline]
    pub fn observe(
        &mut self,
        telemetry: &Telemetry,
        frames: u64,
        callback_ns: Option<u64>,
        elapsed_ns: u64,
        latency_ns: Option<u64>,
    ) {
        let rate = u64::from(self.sample_rate);
        telemetry.callbacks.fetch_add(1, Relaxed);
        let frames32 = u32::try_from(frames).unwrap_or(u32::MAX);
        telemetry.last_block_frames.store(frames32, Relaxed);
        telemetry.max_block_frames.fetch_max(frames32, Relaxed);
        if frames > 0 {
            let budget_ns = frames.saturating_mul(1_000_000_000) / rate;
            if let Some(permille) = elapsed_ns.saturating_mul(1000).checked_div(budget_ns) {
                telemetry
                    .max_load_permille
                    .fetch_max(u32::try_from(permille).unwrap_or(u32::MAX), Relaxed);
                if elapsed_ns > budget_ns {
                    // Processing took longer than the audio it produced: an
                    // underrun is unavoidable unless the backend buffers more.
                    telemetry.late_callbacks.fetch_add(1, Relaxed);
                }
            }
        }
        if let Some(latency) = latency_ns {
            telemetry.output_latency_us.store(latency / 1000, Relaxed);
        }
        if let Some(now) = callback_ns {
            if let Some(last) = self.last_callback_ns {
                let expected = self.last_frames.saturating_mul(1_000_000_000) / rate;
                let actual = now.saturating_sub(last);
                // Heuristic: a gap much longer than the previous block's audio
                // means the device ran without fresh data (likely dropout).
                if expected > 0 && actual > expected.saturating_mul(7) / 4 + 2_000_000 {
                    telemetry.timing_gaps.fetch_add(1, Relaxed);
                }
            }
            self.last_callback_ns = Some(now);
        }
        self.last_frames = frames;
    }
}

/// What the UI and logs receive. All peak values are dBFS (−96 = silence).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TelemetrySnapshot {
    /// Monotonic count of frames rendered since the engine started (all streams).
    #[ts(type = "number")]
    pub clock_frames: u64,
    /// Transport position in frames at the current sample rate.
    #[ts(type = "number")]
    pub transport_frames: u64,
    /// Transport position in project ticks (filled by the session layer).
    #[ts(type = "number | null")]
    pub position_ticks: Option<u64>,
    pub playing: bool,
    pub tone_active: bool,
    pub sample_rate: u32,
    pub output_peak_db: Vec<f32>,
    pub input_peak_db: Vec<f32>,
    pub stats: StreamStats,
}

/// Stream health counters since the current stream was opened.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamStats {
    #[ts(type = "number")]
    pub callbacks: u64,
    #[ts(type = "number")]
    pub input_callbacks: u64,
    /// Callbacks whose processing exceeded the block's real-time budget.
    #[ts(type = "number")]
    pub late_callbacks: u64,
    /// Heuristic timestamp gaps that indicate a probable dropout.
    #[ts(type = "number")]
    pub timing_gaps: u64,
    /// Underruns/overruns reported by the audio backend itself.
    #[ts(type = "number")]
    pub backend_xruns: u64,
    #[ts(type = "number")]
    pub stream_errors: u64,
    #[ts(type = "number")]
    pub clipped_samples: u64,
    /// Highest processing time / block duration ratio seen (1.0 = 100%).
    pub max_callback_load: f32,
    pub last_block_frames: u32,
    pub max_block_frames: u32,
    /// Backend-predicted callback → playback delay, when the backend reports it.
    pub measured_output_latency_ms: Option<f32>,
    /// The OS refused real-time priority for the audio thread.
    pub realtime_denied: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_hold_until_read_and_then_reset() {
        let t = Telemetry::new();
        t.set_stream_format(48_000, 2, 0);
        t.hold_output_peak(0, 0.25);
        t.hold_output_peak(0, -0.5);
        t.hold_output_peak(0, 0.1);
        t.hold_output_peak(1, 1.0);
        t.hold_output_peak(99, 1.0); // out of range: ignored, no panic
        let s = t.snapshot();
        assert!((s.output_peak_db[0] - (-6.0206)).abs() < 1e-3);
        assert!(s.output_peak_db[1].abs() < 1e-6);
        let s = t.snapshot();
        assert_eq!(s.output_peak_db, vec![-96.0, -96.0]);
        assert!(s.input_peak_db.is_empty());
    }

    #[test]
    fn first_fault_wins_and_is_consumed_once() {
        let t = Telemetry::new();
        assert_eq!(t.take_fault(), None);
        t.set_fault(StreamFault::DeviceLost);
        t.set_fault(StreamFault::Fatal);
        assert_eq!(t.take_fault(), Some(StreamFault::DeviceLost));
        assert_eq!(t.take_fault(), None);
    }

    #[test]
    fn monitor_counts_late_callbacks_gaps_and_load() {
        let t = Telemetry::new();
        let mut m = CallbackMonitor::new(48_000);
        // 480 frames = 10 ms budget.
        let block = 10_000_000u64;
        m.observe(&t, 480, Some(0), 2_000_000, Some(5_000_000));
        m.observe(&t, 480, Some(block), 12_000_000, None); // late: 120%
        m.observe(&t, 480, Some(2 * block + 1_000_000), 1_000_000, None); // jitter ok
        m.observe(&t, 480, Some(6 * block), 1_000_000, None); // 38 ms gap
        let s = t.snapshot().stats;
        assert_eq!(s.callbacks, 4);
        assert_eq!(s.late_callbacks, 1);
        assert_eq!(s.timing_gaps, 1);
        assert!((s.max_callback_load - 1.2).abs() < 1e-6);
        assert_eq!(s.max_block_frames, 480);
        assert_eq!(s.measured_output_latency_ms, Some(5.0));
        t.reset_stream_stats();
        assert_eq!(t.snapshot().stats.callbacks, 0);
    }
}
