//! Native effects (Phases 12–13): equaliser, dynamics, utilities, delay,
//! reverbs, modulation, distortion and pitch.
//!
//! Each effect is built on the control thread from an
//! [`elyvon_project::effects::EffectSlot`] (all buffers are allocated in
//! `new`) and processed on the audio thread without allocating, locking or
//! performing IO. Parameters are smoothed ([`Bank`]) so automation and edits
//! never step a gain or a coefficient; a rebuilt effect [`Effect::adopt`]s the
//! running state and the *heard* parameter values of its predecessor, then
//! glides to the new ones.
//!
//! Effects work in control chunks of [`CONTROL`] frames aligned to the
//! absolute timeline frame while the transport runs, so parameter
//! smoothing, LFO phases and tempo-synchronised times do not depend on the
//! callback size: a live pass and an offline bounce compute the same audio.

// Stereo loops index several per-channel arrays at once (`for ch in 0..2`).
#![allow(clippy::needless_range_loop)]

mod delay;
mod distortion;
mod dynamics;
mod eq;
pub mod filters;
mod modulation;
mod pitch;
mod reverb;
mod utility;

pub mod convolution;

pub use eq::{band_responses_db as eq_band_responses_db, response_db as eq_response_db};

use elyvon_core::PPQ;
use elyvon_core::tempo::TempoMap;
use elyvon_project::effects::{EffectKind, EffectSlot, MAX_PARAMS, ParamScale, ParamValues};

/// Frames per parameter/LFO update.
pub const CONTROL: usize = 16;

/// Timeline and routing context of one processed span.
#[derive(Debug, Clone, Copy)]
pub struct FxContext<'a> {
    pub rate: u32,
    pub map: &'a TempoMap,
    /// Absolute timeline frame of the first frame of the buffer.
    pub pos: u64,
    /// The transport plays: `pos` advances with the buffer.
    pub advancing: bool,
    /// The engine is overloaded: expensive effects use their economy path.
    pub economy: bool,
    /// Interleaved stereo key signal for this span (external sidechain).
    pub key: Option<&'a [f32]>,
}

impl<'a> FxContext<'a> {
    /// Musical position in quarter notes at timeline frame `frame`.
    #[inline]
    pub fn quarters_at(&self, frame: u64) -> f64 {
        self.map.tick_at_sample(frame, self.rate) / f64::from(PPQ)
    }

    #[inline]
    pub fn bpm_at(&self, frame: u64) -> f64 {
        let tick = self.map.tick_at_sample(frame, self.rate).max(0.0) as u64;
        self.map.bpm_at(tick)
    }

    /// The context of the part of the buffer starting `offset` frames in.
    pub fn sub(&self, offset: usize, frames: usize) -> FxContext<'a> {
        FxContext {
            pos: if self.advancing {
                self.pos + offset as u64
            } else {
                self.pos
            },
            key: self
                .key
                .and_then(|k| k.get(offset * 2..(offset + frames) * 2)),
            ..*self
        }
    }

    /// Splits `frames` into pieces of control chunks aligned to the
    /// absolute frame grid while advancing. A chunk cut by the end of one
    /// buffer continues in the next call (see [`Bank::enter`]).
    pub fn chunks(&self, frames: usize) -> Chunks {
        Chunks {
            at: 0,
            frames,
            pos: self.pos,
            advancing: self.advancing,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chunks {
    at: usize,
    frames: usize,
    pos: u64,
    advancing: bool,
}

/// A piece of a control chunk inside one buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    /// Offset of the piece in the buffer (frames).
    pub off: usize,
    /// Frames in this piece.
    pub n: usize,
    /// Timeline frame where the control chunk starts.
    pub start: u64,
    /// Frames of the control chunk before this piece.
    pub phase: usize,
    /// Length of the whole control chunk.
    pub len: usize,
}

impl Chunk {
    /// Position of frame `i` of the piece inside its control chunk.
    #[inline]
    pub fn idx(&self, i: usize) -> usize {
        self.phase + i
    }
}

impl Iterator for Chunks {
    type Item = Chunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.at >= self.frames {
            return None;
        }
        let item = if self.advancing {
            let abs = self.pos + self.at as u64;
            let phase = (abs % CONTROL as u64) as usize;
            Chunk {
                off: self.at,
                n: (CONTROL - phase).min(self.frames - self.at),
                start: abs - phase as u64,
                phase,
                len: CONTROL,
            }
        } else {
            let n = CONTROL.min(self.frames - self.at);
            Chunk {
                off: self.at,
                n,
                start: self.pos,
                phase: 0,
                len: n,
            }
        };
        self.at += item.n;
        Some(item)
    }
}

/// Smoothed parameter values of one effect. Continuous parameters glide
/// towards their target with a ~20 ms time constant (in the log domain for
/// logarithmic parameters); choices and toggles change at once.
#[derive(Debug, Clone)]
pub struct Bank {
    kind: EffectKind,
    target: [f64; MAX_PARAMS],
    current: [f64; MAX_PARAMS],
    coef: f64,
    moving: bool,
    /// Control chunk whose values are current (continuation across calls).
    grid: Option<u64>,
}

impl Bank {
    pub fn new(kind: EffectKind, values: &ParamValues, rate: u32) -> Self {
        Self {
            kind,
            target: values.0,
            current: values.0,
            coef: 1.0 - (-(CONTROL as f64) / (0.02 * f64::from(rate.max(1)))).exp(),
            moving: false,
            grid: None,
        }
    }

    /// Enters a piece of a control chunk. Returns `true` when it starts a
    /// new chunk: the parameters glide one step and the effect recomputes
    /// its per-chunk values. A piece that continues the chunk cut by the end
    /// of the previous buffer returns `false` and reuses them, so the output
    /// never depends on where callbacks split the stream.
    pub fn enter(&mut self, c: &Chunk) -> bool {
        if c.phase != 0 && self.grid == Some(c.start) {
            return false;
        }
        self.grid = (c.len == CONTROL).then_some(c.start);
        self.tick();
        true
    }

    /// Current (heard) value.
    #[inline]
    pub fn get(&self, i: usize) -> f64 {
        self.current[i]
    }

    #[inline]
    pub fn target(&self, i: usize) -> f64 {
        self.target[i]
    }

    pub fn set(&mut self, i: usize, value: f64) {
        let Some(def) = self.kind.params().get(i) else {
            return;
        };
        let v = def.clamp(value);
        self.target[i] = v;
        if def.discrete() {
            self.current[i] = v;
        } else if self.current[i] != v {
            self.moving = true;
        }
    }

    /// Advances one control chunk. Returns whether anything moved.
    pub fn tick(&mut self) -> bool {
        if !self.moving {
            return false;
        }
        let mut still = false;
        for (i, def) in self.kind.params().iter().enumerate() {
            let (c, t) = (self.current[i], self.target[i]);
            if c == t || def.discrete() {
                continue;
            }
            let next = if def.scale == ParamScale::Log && c > 0.0 && t > 0.0 {
                (c.ln() + (t.ln() - c.ln()) * self.coef).exp()
            } else {
                c + (t - c) * self.coef
            };
            let range = def.max - def.min;
            if (next - t).abs() <= range * 1e-6 {
                self.current[i] = t;
            } else {
                self.current[i] = next;
                still = true;
            }
        }
        self.moving = still;
        true
    }

    /// Continues from the values the old instance was producing.
    pub fn adopt(&mut self, old: &Bank) {
        if old.kind != self.kind {
            return;
        }
        for (i, def) in self.kind.params().iter().enumerate() {
            if !def.discrete() && def.automatable {
                self.current[i] = old.current[i];
            }
        }
        self.moving = self.current != self.target;
        self.grid = old.grid;
    }
}

/// Linear per-frame interpolation of a value over one control chunk.
#[derive(Debug, Clone, Copy)]
pub struct Ramp {
    pub from: f32,
    pub to: f32,
}

impl Ramp {
    /// A value that is not moving.
    pub const fn hold(v: f32) -> Self {
        Self { from: v, to: v }
    }

    /// On a new control chunk, ramps from the previous target to `to`.
    /// Returns the ramp of the current chunk.
    #[inline]
    pub fn step(&mut self, fresh: bool, to: f32) -> Ramp {
        if fresh {
            self.from = self.to;
            self.to = to;
        }
        *self
    }

    #[inline]
    pub fn at(&self, i: usize, n: usize) -> f32 {
        if self.from == self.to {
            return self.to;
        }
        self.from + (self.to - self.from) * (i + 1) as f32 / n.max(1) as f32
    }
}

/// LFO phase that follows the song position: tempo-synchronised LFOs are
/// locked to the musical position (and so to the tempo map); free LFOs
/// accumulate their rate and re-anchor to the timeline after a jump. While
/// the transport is stopped they free-run.
#[derive(Debug, Clone, Copy, Default)]
pub struct LfoClock {
    phase: f64,
    expected: Option<u64>,
}

impl LfoClock {
    /// Phase and per-frame increment of the control chunk `c` (call once
    /// per new chunk): `begin` then `end` over the whole chunk.
    pub fn chunk(
        &mut self,
        cx: &FxContext<'_>,
        c: &Chunk,
        hz: f64,
        sync: Option<f64>,
    ) -> (f64, f64) {
        let (p, inc) = self.begin(cx, c.start, hz, sync);
        self.end(cx, c.start, c.len, inc);
        (p, inc)
    }

    /// Phase (0…1) at the start of a chunk at timeline frame `abs` and its
    /// increment per frame. `sync` is the cycle length in quarter notes.
    pub fn begin(
        &mut self,
        cx: &FxContext<'_>,
        abs: u64,
        hz: f64,
        sync: Option<f64>,
    ) -> (f64, f64) {
        let rate = f64::from(cx.rate.max(1));
        match sync {
            Some(quarters) if cx.advancing => {
                let q = quarters.max(1e-6);
                let phase = (cx.quarters_at(abs) / q).rem_euclid(1.0);
                let inc = cx.bpm_at(abs) / 60.0 / q / rate;
                self.phase = phase;
                (phase, inc)
            }
            Some(quarters) => {
                let inc = cx.bpm_at(cx.pos) / 60.0 / quarters.max(1e-6) / rate;
                self.expected = None;
                (self.phase, inc)
            }
            None => {
                if cx.advancing && self.expected != Some(abs) {
                    self.phase = (abs as f64 / rate * hz).rem_euclid(1.0);
                }
                (self.phase, hz / rate)
            }
        }
    }

    /// Ends a chunk of `n` frames started with `begin`.
    pub fn end(&mut self, cx: &FxContext<'_>, abs: u64, n: usize, inc: f64) {
        self.phase = (self.phase + inc * n as f64).rem_euclid(1.0);
        self.expected = cx.advancing.then_some(abs + n as u64);
    }
}

/// Denormal guard for recursive state.
#[inline]
pub fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 { 0.0 } else { x }
}

#[inline]
pub fn flush64(x: f64) -> f64 {
    if x.abs() < 1e-30 { 0.0 } else { x }
}

#[inline]
pub fn db(x: f64) -> f64 {
    10f64.powf(x / 20.0)
}

/// One-pole smoothing coefficient for a time constant in milliseconds.
#[inline]
pub fn time_coef(ms: f64, rate: u32) -> f64 {
    let frames = ms * 0.001 * f64::from(rate.max(1));
    if frames <= 0.0 {
        0.0
    } else {
        (-1.0 / frames).exp()
    }
}

/// Peak of the last processed buffer below which an effect counts as
/// silent (−110 dBFS).
pub const SILENT: f32 = 3.2e-6;

/// A prepared native effect. Always boxed in an insert, so the size of the
/// largest variant (the equaliser's parameter bank) does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Effect {
    Eq(eq::ParametricEq),
    Compressor(dynamics::Compressor),
    Gate(dynamics::Gate),
    StereoTool(utility::StereoTool),
    Generator(utility::Generator),
    Delay(delay::Delay),
    Reverb(reverb::Reverb),
    Convolution(convolution::ConvolutionReverb),
    Chorus(modulation::Chorus),
    Flanger(modulation::Flanger),
    Phaser(modulation::Phaser),
    Tremolo(modulation::Tremolo),
    Rotary(modulation::Rotary),
    Distortion(distortion::Distortion),
    Bitcrusher(distortion::Bitcrusher),
    Pitch(pitch::PitchShifter),
}

macro_rules! each {
    ($self:expr, $e:ident => $body:expr) => {
        match $self {
            Effect::Eq($e) => $body,
            Effect::Compressor($e) => $body,
            Effect::Gate($e) => $body,
            Effect::StereoTool($e) => $body,
            Effect::Generator($e) => $body,
            Effect::Delay($e) => $body,
            Effect::Reverb($e) => $body,
            Effect::Convolution($e) => $body,
            Effect::Chorus($e) => $body,
            Effect::Flanger($e) => $body,
            Effect::Phaser($e) => $body,
            Effect::Tremolo($e) => $body,
            Effect::Rotary($e) => $body,
            Effect::Distortion($e) => $body,
            Effect::Bitcrusher($e) => $body,
            Effect::Pitch($e) => $body,
        }
    };
}

impl Effect {
    /// Control thread: allocates every buffer the effect will use.
    pub fn new(slot: &EffectSlot, rate: u32) -> Self {
        let p = &slot.params;
        match slot.kind {
            EffectKind::ParametricEq => Self::Eq(eq::ParametricEq::new(p, rate)),
            EffectKind::Compressor => Self::Compressor(dynamics::Compressor::new(p, rate)),
            EffectKind::Gate => Self::Gate(dynamics::Gate::new(p, rate)),
            EffectKind::StereoTool => Self::StereoTool(utility::StereoTool::new(p, rate)),
            EffectKind::TestGenerator => Self::Generator(utility::Generator::new(p, rate)),
            EffectKind::Delay => Self::Delay(delay::Delay::new(p, rate)),
            EffectKind::Reverb => Self::Reverb(reverb::Reverb::new(p, rate)),
            EffectKind::ConvolutionReverb => {
                Self::Convolution(convolution::ConvolutionReverb::new(p, rate))
            }
            EffectKind::Chorus => Self::Chorus(modulation::Chorus::new(p, rate)),
            EffectKind::Flanger => Self::Flanger(modulation::Flanger::new(p, rate)),
            EffectKind::Phaser => Self::Phaser(modulation::Phaser::new(p, rate)),
            EffectKind::Tremolo => Self::Tremolo(modulation::Tremolo::new(p, rate)),
            EffectKind::Rotary => Self::Rotary(modulation::Rotary::new(p, rate)),
            EffectKind::Distortion => Self::Distortion(distortion::Distortion::new(p, rate)),
            EffectKind::Bitcrusher => Self::Bitcrusher(distortion::Bitcrusher::new(p, rate)),
            EffectKind::PitchShifter => Self::Pitch(pitch::PitchShifter::new(p, rate)),
        }
    }

    pub fn kind(&self) -> EffectKind {
        match self {
            Self::Eq(_) => EffectKind::ParametricEq,
            Self::Compressor(_) => EffectKind::Compressor,
            Self::Gate(_) => EffectKind::Gate,
            Self::StereoTool(_) => EffectKind::StereoTool,
            Self::Generator(_) => EffectKind::TestGenerator,
            Self::Delay(_) => EffectKind::Delay,
            Self::Reverb(_) => EffectKind::Reverb,
            Self::Convolution(_) => EffectKind::ConvolutionReverb,
            Self::Chorus(_) => EffectKind::Chorus,
            Self::Flanger(_) => EffectKind::Flanger,
            Self::Phaser(_) => EffectKind::Phaser,
            Self::Tremolo(_) => EffectKind::Tremolo,
            Self::Rotary(_) => EffectKind::Rotary,
            Self::Distortion(_) => EffectKind::Distortion,
            Self::Bitcrusher(_) => EffectKind::Bitcrusher,
            Self::Pitch(_) => EffectKind::PitchShifter,
        }
    }

    /// Sets a parameter target (automation). Real-time safe.
    pub fn set(&mut self, index: usize, value: f64) {
        each!(self, e => e.bank.set(index, value))
    }

    /// Processes interleaved stereo `buf` in place. Real-time safe.
    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        each!(self, e => e.process(buf, cx));
        let peak = buf.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        self.set_out_peak(peak);
    }

    fn set_out_peak(&mut self, peak: f32) {
        each!(self, e => e.out_peak = peak)
    }

    /// The output of the last processed buffer was audible: the effect may
    /// still be ringing (delay and reverb tails) and should keep running
    /// while the transport is stopped.
    pub fn ringing(&self) -> bool {
        each!(self, e => e.out_peak > SILENT)
    }

    /// Frames of delay the effect adds to the signal it processes.
    pub fn latency(&self) -> usize {
        match self {
            Self::Convolution(c) => c.latency(),
            Self::Distortion(d) => d.latency(),
            _ => 0,
        }
    }

    /// Largest gain reduction (dB, ≥ 0) since the previous call, for the
    /// dynamics meters.
    pub fn take_reduction(&mut self) -> Option<f32> {
        match self {
            Self::Compressor(c) => Some(std::mem::take(&mut c.reduction)),
            Self::Gate(g) => Some(std::mem::take(&mut g.reduction)),
            _ => None,
        }
    }

    /// Continues the old instance's state and heard parameter values.
    pub fn adopt(&mut self, old: &mut Effect) {
        match (self, old) {
            (Self::Eq(a), Self::Eq(b)) => a.adopt(b),
            (Self::Compressor(a), Self::Compressor(b)) => a.adopt(b),
            (Self::Gate(a), Self::Gate(b)) => a.adopt(b),
            (Self::StereoTool(a), Self::StereoTool(b)) => a.adopt(b),
            (Self::Generator(a), Self::Generator(b)) => a.adopt(b),
            (Self::Delay(a), Self::Delay(b)) => a.adopt(b),
            (Self::Reverb(a), Self::Reverb(b)) => a.adopt(b),
            (Self::Convolution(a), Self::Convolution(b)) => a.adopt(b),
            (Self::Chorus(a), Self::Chorus(b)) => a.adopt(b),
            (Self::Flanger(a), Self::Flanger(b)) => a.adopt(b),
            (Self::Phaser(a), Self::Phaser(b)) => a.adopt(b),
            (Self::Tremolo(a), Self::Tremolo(b)) => a.adopt(b),
            (Self::Rotary(a), Self::Rotary(b)) => a.adopt(b),
            (Self::Distortion(a), Self::Distortion(b)) => a.adopt(b),
            (Self::Bitcrusher(a), Self::Bitcrusher(b)) => a.adopt(b),
            (Self::Pitch(a), Self::Pitch(b)) => a.adopt(b),
            _ => {}
        }
    }

    /// Relative processing cost per frame (1 ≈ one biquad on two channels),
    /// for CPU budgeting and the effect editor.
    pub fn cost(&self) -> f32 {
        match self {
            Self::Eq(_) => 10.0,
            Self::Compressor(_) | Self::Gate(_) => 4.0,
            Self::StereoTool(_) | Self::Generator(_) | Self::Tremolo(_) => 2.0,
            Self::Delay(_) => 6.0,
            Self::Reverb(_) => 30.0,
            Self::Convolution(c) => c.cost(),
            Self::Chorus(_) | Self::Flanger(_) | Self::Phaser(_) => 8.0,
            Self::Rotary(_) => 12.0,
            Self::Distortion(d) => d.cost(),
            Self::Bitcrusher(_) => 2.0,
            Self::Pitch(_) => 6.0,
        }
    }
}

/// Latency in frames of `slot`, without building it (for the UI).
pub fn effect_latency(slot: &EffectSlot) -> usize {
    match slot.kind {
        EffectKind::ConvolutionReverb => convolution::PARTITION,
        EffectKind::Distortion => distortion::latency_for(&slot.params),
        _ => 0,
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use elyvon_core::{Tempo, TimeSignature};

    pub fn map(bpm: f64) -> TempoMap {
        TempoMap::constant(Tempo::new(bpm).expect("tempo"), TimeSignature::default())
    }

    pub fn cx(map: &TempoMap, rate: u32, pos: u64) -> FxContext<'_> {
        FxContext {
            rate,
            map,
            pos,
            advancing: true,
            economy: false,
            key: None,
        }
    }

    pub fn sine(freq: f64, rate: u32, frames: usize, amp: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = amp
                    * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin() as f32;
                [v, v]
            })
            .collect()
    }

    pub fn rms(x: &[f32]) -> f32 {
        if x.is_empty() {
            return 0.0;
        }
        (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt()
            as f32
    }

    /// Runs `fx` over `input` in blocks of `block` frames, advancing the timeline.
    pub fn run(
        fx: &mut Effect,
        input: &[f32],
        rate: u32,
        block: usize,
        map: &TempoMap,
    ) -> Vec<f32> {
        let mut out = input.to_vec();
        let mut pos = 0u64;
        for piece in out.chunks_mut(block * 2) {
            let c = cx(map, rate, pos);
            fx.process(piece, &c);
            pos += (piece.len() / 2) as u64;
        }
        out
    }

    pub fn slot(kind: EffectKind, values: &[(&str, f64)]) -> EffectSlot {
        let mut s = EffectSlot::new(kind);
        for (k, v) in values {
            s.set(k, *v).expect("known key");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use elyvon_project::effects::ALL_EFFECTS;

    #[test]
    fn chunks_follow_the_absolute_grid() {
        let m = map(120.0);
        let c = cx(&m, 48_000, 10);
        let v: Vec<_> = c
            .chunks(40)
            .map(|c| (c.off, c.n, c.start, c.phase))
            .collect();
        assert_eq!(
            v,
            vec![
                (0, 6, 0, 10),
                (6, 16, 16, 0),
                (22, 16, 32, 0),
                (38, 2, 48, 0)
            ]
        );
    }

    #[test]
    fn bank_glides_and_adopts_the_heard_value() {
        let s = slot(EffectKind::Compressor, &[]);
        let mut b = Bank::new(EffectKind::Compressor, &s.params, 48_000);
        b.set(0, -40.0);
        b.tick();
        let mid = b.get(0);
        assert!(mid < -18.0 && mid > -40.0, "{mid}");
        for _ in 0..2000 {
            b.tick();
        }
        assert_eq!(b.get(0), -40.0);
        let mut fresh = Bank::new(EffectKind::Compressor, &s.params, 48_000);
        let mut old = Bank::new(EffectKind::Compressor, &s.params, 48_000);
        old.set(0, -50.0);
        old.tick();
        fresh.adopt(&old);
        assert!(fresh.get(0) < -18.0, "ramps from the old value");
    }

    /// Every effect, at default and extreme settings, at the common block
    /// sizes: no NaN/inf, bounded output.
    #[test]
    fn every_effect_stays_finite_and_bounded_at_extremes() {
        let rate = 48_000;
        let m = map(120.0);
        let input = sine(220.0, rate, 9_600, 0.9);
        for kind in ALL_EFFECTS {
            for extreme in [None, Some(false), Some(true)] {
                let mut s = EffectSlot::new(*kind);
                if let Some(high) = extreme {
                    for (i, d) in kind.params().iter().enumerate() {
                        s.params.0[i] = if high { d.max } else { d.min };
                    }
                }
                for block in [32, 64, 128, 333] {
                    let mut fx = Effect::new(&s, rate);
                    let out = run(&mut fx, &input, rate, block, &m);
                    assert!(
                        out.iter().all(|v| v.is_finite() && v.abs() < 64.0),
                        "{kind:?} {extreme:?} block {block}: max {}",
                        out.iter().fold(0f32, |a, v| a.max(v.abs()))
                    );
                }
            }
        }
    }

    /// Automating every parameter between its extremes every chunk keeps
    /// every effect finite.
    #[test]
    fn automation_stress_keeps_every_effect_finite() {
        let rate = 44_100;
        let m = map(97.0);
        let input = sine(330.0, rate, 8_192, 1.0);
        for kind in ALL_EFFECTS {
            for block in [32, 64, 128] {
                let mut fx = Effect::new(&EffectSlot::new(*kind), rate);
                let mut out = input.clone();
                let mut pos = 0u64;
                for (k, piece) in out.chunks_mut(block * 2).enumerate() {
                    for (i, d) in kind.params().iter().enumerate() {
                        if d.automatable {
                            let v = if (k + i) % 2 == 0 { d.max } else { d.min };
                            fx.set(i, v);
                        }
                    }
                    fx.process(piece, &cx(&m, rate, pos));
                    pos += (piece.len() / 2) as u64;
                }
                assert!(
                    out.iter().all(|v| v.is_finite() && v.abs() < 64.0),
                    "{kind:?} block {block}"
                );
            }
        }
    }

    /// The same settings give bit-identical audio whatever the callback
    /// size, also while parameters glide (live playback equals an offline
    /// bounce).
    #[test]
    fn output_does_not_depend_on_the_block_size() {
        let rate = 48_000;
        let m = map(128.0);
        let input = sine(440.0, rate, 6_000, 0.5);
        for kind in ALL_EFFECTS {
            let s = EffectSlot::new(*kind);
            let a = run(&mut Effect::new(&s, rate), &input, rate, 37, &m);
            let b = run(&mut Effect::new(&s, rate), &input, rate, 128, &m);
            assert_eq!(a, b, "{kind:?}");
            // A parameter change applied at the same frame glides identically.
            let glide = |block: usize| {
                let mut fx = Effect::new(&s, rate);
                let mut out = input.clone();
                let (head, tail) = out.split_at_mut(1_024 * 2);
                let mut pos = 0u64;
                for piece in head.chunks_mut(block * 2) {
                    fx.process(piece, &cx(&m, rate, pos));
                    pos += (piece.len() / 2) as u64;
                }
                for (i, d) in kind.params().iter().enumerate() {
                    if d.automatable && !d.discrete() {
                        fx.set(i, d.min + (d.max - d.min) * 0.3);
                    }
                }
                for piece in tail.chunks_mut(block * 2) {
                    fx.process(piece, &cx(&m, rate, pos));
                    pos += (piece.len() / 2) as u64;
                }
                out
            };
            assert_eq!(glide(37), glide(128), "{kind:?} while gliding");
        }
    }

    /// Every effect, at its default setting, measurably changes a test signal
    /// (or, for the neutral equaliser and stereo tool, passes it unchanged).
    #[test]
    fn every_effect_changes_the_signal_measurably() {
        let rate = 48_000;
        let m = map(120.0);
        let mut input = sine(500.0, rate, 24_000, 0.5);
        // Stereo difference and a loud section so every effect has work.
        for (i, f) in input.chunks_mut(2).enumerate() {
            f[1] *= 0.6;
            if i > 12_000 {
                f[0] *= 1.8;
                f[1] *= 1.8;
            }
        }
        let changed = |kind: EffectKind, values: &[(&str, f64)]| {
            let s = slot(kind, values);
            let out = run(&mut Effect::new(&s, rate), &input, rate, 128, &m);
            let diff: Vec<f32> = out.iter().zip(&input).map(|(a, b)| a - b).collect();
            rms(&diff) / rms(&input)
        };
        let cases: &[(EffectKind, &[(&str, f64)])] = &[
            (
                EffectKind::ParametricEq,
                &[("b4Gain", 9.0), ("b4Freq", 500.0)],
            ),
            (EffectKind::Compressor, &[("thresholdDb", -30.0)]),
            (EffectKind::Gate, &[("thresholdDb", -5.0)]),
            (EffectKind::StereoTool, &[("width", 0.0)]),
            (EffectKind::TestGenerator, &[]),
            (EffectKind::Delay, &[]),
            (EffectKind::Reverb, &[]),
            (EffectKind::ConvolutionReverb, &[("lengthSeconds", 0.5)]),
            (EffectKind::Chorus, &[]),
            (EffectKind::Flanger, &[]),
            (EffectKind::Phaser, &[]),
            (EffectKind::Tremolo, &[]),
            (EffectKind::Rotary, &[]),
            (EffectKind::Distortion, &[]),
            (EffectKind::Bitcrusher, &[]),
            (EffectKind::PitchShifter, &[("semitones", 5.0)]),
        ];
        assert_eq!(cases.len(), ALL_EFFECTS.len());
        for (kind, values) in cases {
            let c = changed(*kind, values);
            assert!(c > 0.05, "{kind:?} changes the signal by {c}");
        }
        assert!(
            changed(EffectKind::ParametricEq, &[]) < 1e-5,
            "neutral EQ is transparent"
        );
        assert!(
            changed(EffectKind::StereoTool, &[]) < 1e-6,
            "neutral utility is transparent"
        );
    }

    #[test]
    fn sample_rates_scale_times_and_frequencies() {
        // A 100 ms, no-feedback delay lands 100 ms later at every rate.
        for rate in [44_100u32, 48_000, 88_200, 96_000, 192_000] {
            let m = map(120.0);
            let s = slot(
                EffectKind::Delay,
                &[
                    ("sync", 0.0),
                    ("timeLeftMs", 100.0),
                    ("timeRightMs", 100.0),
                    ("feedback", 0.0),
                    ("mix", 100.0),
                    ("lowCutHz", 20.0),
                    ("highCutHz", 20_000.0),
                ],
            );
            let mut fx = Effect::new(&s, rate);
            let n = rate as usize / 4;
            let mut x = vec![0.0f32; n * 2];
            x[0] = 1.0;
            x[1] = 1.0;
            let out = run(&mut fx, &x, rate, 128, &m);
            let peak = out
                .chunks(2)
                .enumerate()
                .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
                .map(|(i, _)| i)
                .unwrap_or(0);
            let expected = (0.1 * f64::from(rate)).round() as usize;
            assert!(peak.abs_diff(expected) <= 2, "{rate}: {peak} vs {expected}");
        }
    }
}
