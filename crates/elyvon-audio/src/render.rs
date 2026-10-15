//! The real-time engine core: everything the audio callback runs.
//!
//! [`EngineCore`] is created on a control thread (where it allocates its
//! scratch buffers once) and then **moved** into the device callback. From
//! then on every method reachable from [`EngineCore::process_interleaved`] /
//! [`EngineCore::process_planar`] is real-time safe: no allocation or
//! deallocation, no locks, no I/O, no logging and no system calls. This is
//! enforced by the allocation-counting test in `tests/realtime.rs`.
//!
//! Per block the core:
//! 1. applies at most [`MAX_COMMANDS_PER_BLOCK`] queued commands,
//! 2. renders silence plus the opt-in test tone into planar `f32` scratch,
//! 3. measures per-channel peaks (before clamping, so overs are visible),
//! 4. converts/clamps into the device format and counts clipped samples,
//! 5. advances the monotonic sample clock and, if playing, the transport,
//! 6. publishes clocks to [`Telemetry`].
//!
//! Device callbacks of any size are split into chunks of at most
//! [`MAX_BLOCK_FRAMES`], so the output is identical for every buffer size.

use std::sync::Arc;

use crate::command::{Command, CommandReceiver, MAX_COMMANDS_PER_BLOCK, TestTone, ToneChannels};
use crate::format::{OutputSample, interleave_into};
use crate::gain::db_to_gain;
use crate::telemetry::Telemetry;

/// Internal processing chunk. Device blocks larger than this are split.
pub const MAX_BLOCK_FRAMES: usize = 1024;
/// Output channels rendered by the engine; extra device channels get silence.
pub const MAX_ENGINE_CHANNELS: usize = 32;
/// Test-tone gain ramp, avoids clicks when it is switched on or off.
pub const TONE_RAMP_SECONDS: f64 = 0.010;

/// Construction parameters for [`EngineCore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreConfig {
    pub sample_rate: u32,
    /// Engine output channels (clamped to 1…[`MAX_ENGINE_CHANNELS`]).
    pub output_channels: u16,
    /// Initial value of the monotonic clock (keeps it monotonic across streams).
    pub start_clock: u64,
    /// Initial transport position in frames.
    pub start_transport: u64,
}

#[derive(Debug)]
struct Tone {
    settings: TestTone,
    phase: f64,
    increment: f64,
    gain: f32,
    target: f32,
    step: f32,
}

impl Tone {
    fn new(sample_rate: u32) -> Self {
        let mut tone = Self {
            settings: TestTone::default(),
            phase: 0.0,
            increment: 0.0,
            gain: 0.0,
            target: 0.0,
            step: 0.0,
        };
        tone.apply(TestTone::default(), sample_rate);
        tone
    }

    /// Pure arithmetic, no allocation: safe on the audio thread.
    fn apply(&mut self, settings: TestTone, sample_rate: u32) {
        let mut settings = settings.sanitized(sample_rate);
        if !settings.enabled && self.audible() {
            // Fade out the tone that is sounding, at its own pitch and channels.
            settings.frequency_hz = self.settings.frequency_hz;
            settings.channels = self.settings.channels;
        }
        self.settings = settings;
        self.increment = f64::from(settings.frequency_hz) / f64::from(sample_rate.max(1));
        self.target = if settings.enabled {
            db_to_gain(f64::from(settings.level_db)) as f32
        } else {
            0.0
        };
        let ramp_frames = (TONE_RAMP_SECONDS * f64::from(sample_rate)).max(1.0) as f32;
        self.step = (self.target - self.gain).abs() / ramp_frames;
    }

    fn audible(&self) -> bool {
        self.gain > 0.0 || self.target > 0.0
    }

    #[inline]
    fn next_gain(&mut self) -> f32 {
        if self.gain < self.target {
            self.gain = (self.gain + self.step).min(self.target);
        } else if self.gain > self.target {
            self.gain = (self.gain - self.step).max(self.target);
        }
        self.gain
    }
}

/// Real-time engine core. See the module documentation for the rules.
#[derive(Debug)]
pub struct EngineCore {
    sample_rate: u32,
    channels: usize,
    /// Planar scratch: `channels` × [`MAX_BLOCK_FRAMES`], allocated once.
    scratch: Vec<f32>,
    commands: CommandReceiver,
    telemetry: Arc<Telemetry>,
    clock: u64,
    transport: u64,
    playing: bool,
    tone: Tone,
}

impl EngineCore {
    /// Allocates all buffers. Control thread only.
    pub fn new(config: CoreConfig, commands: CommandReceiver, telemetry: Arc<Telemetry>) -> Self {
        let channels = usize::from(config.output_channels).clamp(1, MAX_ENGINE_CHANNELS);
        let sample_rate = config.sample_rate.max(1);
        telemetry.publish_clock(config.start_clock, config.start_transport, false, false);
        Self {
            sample_rate,
            channels,
            scratch: vec![0.0; channels * MAX_BLOCK_FRAMES],
            commands,
            telemetry,
            clock: config.start_clock,
            transport: config.start_transport,
            playing: false,
            tone: Tone::new(sample_rate),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn clock(&self) -> u64 {
        self.clock
    }

    pub fn transport(&self) -> u64 {
        self.transport
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    fn apply_commands(&mut self) {
        for _ in 0..MAX_COMMANDS_PER_BLOCK {
            let Some(command) = self.commands.pop() else {
                break;
            };
            match command {
                Command::Play => self.playing = true,
                Command::Stop => self.playing = false,
                Command::Locate { frame } => self.transport = frame,
                Command::SetTestTone(tone) => self.tone.apply(tone, self.sample_rate),
            }
        }
    }

    /// Renders `frames` (≤ [`MAX_BLOCK_FRAMES`]) into the scratch buffer and
    /// meters it. Real-time safe.
    fn render_chunk(&mut self, frames: usize) {
        debug_assert!(frames <= MAX_BLOCK_FRAMES);
        self.apply_commands();
        let channels = self.channels;
        // 1. Silence: the engine always produces a defined signal.
        for channel in self.scratch.as_chunks_mut::<MAX_BLOCK_FRAMES>().0 {
            channel[..frames].fill(0.0);
        }
        // 2. Opt-in test tone.
        if self.tone.audible() {
            let target_channel = match self.tone.settings.channels {
                ToneChannels::All => None,
                ToneChannels::Only { channel } => Some(usize::from(channel)),
            };
            for frame in 0..frames {
                let gain = self.tone.next_gain();
                let value = ((self.tone.phase * std::f64::consts::TAU).sin() as f32) * gain;
                self.tone.phase += self.tone.increment;
                if self.tone.phase >= 1.0 {
                    self.tone.phase -= 1.0;
                }
                for channel in 0..channels {
                    if target_channel.is_none_or(|c| c == channel) {
                        self.scratch[channel * MAX_BLOCK_FRAMES + frame] += value;
                    }
                }
            }
            if !self.tone.audible() {
                self.tone.phase = 0.0;
            }
        }
        // 3. Meters (pre-clamp so overs are visible).
        for (channel, samples) in self
            .scratch
            .as_chunks::<MAX_BLOCK_FRAMES>()
            .0
            .iter()
            .enumerate()
        {
            let peak = samples[..frames].iter().fold(0.0f32, |peak, v| {
                if v.is_nan() { 1.0 } else { peak.max(v.abs()) }
            });
            self.telemetry.hold_output_peak(channel, peak);
        }
    }

    fn advance(&mut self, frames: usize) {
        let frames = frames as u64;
        self.clock = self.clock.saturating_add(frames);
        if self.playing {
            self.transport = self.transport.saturating_add(frames);
        }
    }

    fn publish(&self) {
        self.telemetry.publish_clock(
            self.clock,
            self.transport,
            self.playing,
            self.tone.audible(),
        );
    }

    /// Fills an interleaved device buffer of `device_channels` channels.
    /// Real-time safe for every [`OutputSample`] type.
    pub fn process_interleaved<S: OutputSample>(&mut self, out: &mut [S], device_channels: usize) {
        if device_channels == 0 {
            return;
        }
        let total = out.len() / device_channels;
        let mut done = 0;
        let mut clipped = 0;
        while done < total {
            let frames = (total - done).min(MAX_BLOCK_FRAMES);
            self.render_chunk(frames);
            let mut planar: [&[f32]; MAX_ENGINE_CHANNELS] = [&[]; MAX_ENGINE_CHANNELS];
            for (slot, channel) in planar
                .iter_mut()
                .zip(self.scratch.as_chunks::<MAX_BLOCK_FRAMES>().0)
            {
                *slot = &channel[..frames];
            }
            let start = done * device_channels;
            clipped += interleave_into(
                &planar[..self.channels],
                frames,
                &mut out[start..start + frames * device_channels],
                device_channels,
            );
            self.advance(frames);
            done += frames;
        }
        // Samples left over by a buffer that is not a whole number of frames.
        for s in &mut out[total * device_channels..] {
            *s = S::SILENCE;
        }
        self.telemetry.add_clipped(clipped);
        self.publish();
    }

    /// Fills planar `f32` output buffers (one slice per channel, equal length).
    /// Values are clamped exactly like a device stream. Real-time safe.
    pub fn process_planar(&mut self, outputs: &mut [&mut [f32]]) {
        let total = outputs.iter().map(|c| c.len()).min().unwrap_or(0);
        let mut done = 0;
        let mut clipped = 0u64;
        while done < total {
            let frames = (total - done).min(MAX_BLOCK_FRAMES);
            self.render_chunk(frames);
            for (channel, out) in outputs.iter_mut().enumerate() {
                let dst = &mut out[done..done + frames];
                if channel < self.channels {
                    let src = &self.scratch[channel * MAX_BLOCK_FRAMES..][..frames];
                    for (d, s) in dst.iter_mut().zip(src) {
                        let (v, was_clipped) = crate::format::clamp_unit(*s);
                        clipped += u64::from(was_clipped);
                        *d = v;
                    }
                } else {
                    dst.fill(0.0);
                }
            }
            self.advance(frames);
            done += frames;
        }
        self.telemetry.add_clipped(clipped);
        self.publish();
    }
}
