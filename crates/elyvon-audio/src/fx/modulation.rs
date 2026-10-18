//! Modulation effects: chorus, flanger, phaser, tremolo/auto-pan and a
//! rotary-speaker simulation.
//!
//! LFOs use [`LfoClock`]: synchronised rates are locked to the musical
//! position (so they follow the tempo map and land on the beat in every
//! pass), free rates re-anchor to the timeline after a locate. Recursive
//! paths (flanger and chorus feedback, phaser feedback) keep their loop
//! gain below one and read delay lines linearly.

use elyvon_project::effects::{EffectKind, ParamValues, division_quarters};

use super::delay::{Line, balanced_mix};
use super::filters::{Biquad, Coefs, butterworth_q};
use super::{Bank, FxContext, LfoClock, Ramp};

use std::f64::consts::TAU;

/// Rate parameters shared by the synchronisable LFO effects.
fn lfo_rate(bank: &Bank, rate: usize, sync: usize, division: usize) -> (f64, Option<f64>) {
    let hz = bank.get(rate);
    let synced =
        (bank.get(sync) >= 0.5).then(|| division_quarters(bank.get(division).round() as usize));
    (hz, synced)
}

#[inline]
fn sine01(phase: f64) -> f32 {
    (0.5 + 0.5 * (TAU * phase).sin()) as f32
}

mod chp {
    pub const RATE: usize = 0;
    pub const SYNC: usize = 1;
    pub const DIVISION: usize = 2;
    pub const DEPTH: usize = 3;
    pub const DELAY: usize = 4;
    pub const VOICES: usize = 5;
    pub const FEEDBACK: usize = 6;
    pub const SPREAD: usize = 7;
    pub const MIX: usize = 8;
}

#[derive(Debug, Clone)]
pub struct Chorus {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    line: Line,
    clock: LfoClock,
    /// Phase and increment of the current control chunk.
    lfo: (f64, f64),
    mix: [Ramp; 2],
    last: [f32; 2],
}

impl Chorus {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        let mut c = Self {
            bank: Bank::new(EffectKind::Chorus, p, rate),
            out_peak: 0.0,
            rate,
            line: Line::new((0.06 * f64::from(rate)) as usize + 8),
            clock: LfoClock::default(),
            lfo: (0.0, 0.0),
            mix: [Ramp::hold(0.0); 2],
            last: [0.0; 2],
        };
        c.mix = c.mix_gains().map(Ramp::hold);
        c
    }

    fn mix_gains(&self) -> [f32; 2] {
        let (d, w) = balanced_mix((self.bank.get(chp::MIX) / 100.0) as f32);
        [d, w]
    }

    pub fn adopt(&mut self, old: &mut Chorus) {
        self.bank.adopt(&old.bank);
        self.line.adopt(&mut old.line);
        self.clock = old.clock;
        self.lfo = old.lfo;
        self.mix = old.mix;
        self.last = old.last;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let ms = 0.001 * f64::from(self.rate);
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            if fresh {
                let (hz, sync) = lfo_rate(&self.bank, chp::RATE, chp::SYNC, chp::DIVISION);
                self.lfo = self.clock.chunk(cx, &ck, hz, sync);
            }
            let (phase0, inc) = self.lfo;
            let b = &self.bank;
            let voices = b.get(chp::VOICES).round() as usize + 1;
            let base = (b.get(chp::DELAY) * ms) as f32;
            let depth = (b.get(chp::DEPTH) * ms) as f32;
            let fb = (b.get(chp::FEEDBACK) / 100.0) as f32;
            let spread = b.get(chp::SPREAD) / 100.0 * 0.25;
            let target = self.mix_gains();
            let dry = self.mix[0].step(fresh, target[0]);
            let wet = self.mix[1].step(fresh, target[1]);
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let j = ck.idx(i);
                let phase = phase0 + inc * j as f64;
                let mut out = [0.0f32; 2];
                for (ch, o) in out.iter_mut().enumerate() {
                    for v in 0..voices {
                        let p =
                            phase + v as f64 / voices as f64 + if ch == 1 { spread } else { 0.0 };
                        let d = base + depth * sine01(p);
                        *o += self.line.read_linear(d, ch);
                    }
                    *o /= voices as f32;
                }
                self.line
                    .write([f[0] + self.last[0] * fb, f[1] + self.last[1] * fb]);
                self.last = out;
                let (dg, wg) = (dry.at(j, ck.len), wet.at(j, ck.len));
                f[0] = f[0] * dg + out[0] * wg;
                f[1] = f[1] * dg + out[1] * wg;
            }
            self.last = [super::flush(self.last[0]), super::flush(self.last[1])];
        }
    }
}

mod flp {
    pub const RATE: usize = 0;
    pub const SYNC: usize = 1;
    pub const DIVISION: usize = 2;
    pub const DEPTH: usize = 3;
    pub const DELAY: usize = 4;
    pub const FEEDBACK: usize = 5;
    pub const SPREAD: usize = 6;
    pub const MIX: usize = 7;
}

#[derive(Debug, Clone)]
pub struct Flanger {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    line: Line,
    clock: LfoClock,
    lfo: (f64, f64),
    last: [f32; 2],
    mix: Ramp,
}

impl Flanger {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        Self {
            bank: Bank::new(EffectKind::Flanger, p, rate),
            out_peak: 0.0,
            rate,
            line: Line::new((0.02 * f64::from(rate)) as usize + 8),
            clock: LfoClock::default(),
            lfo: (0.0, 0.0),
            last: [0.0; 2],
            mix: Ramp::hold((p.get(flp::MIX) / 100.0) as f32),
        }
    }

    pub fn adopt(&mut self, old: &mut Flanger) {
        self.bank.adopt(&old.bank);
        self.line.adopt(&mut old.line);
        self.clock = old.clock;
        self.lfo = old.lfo;
        self.last = old.last;
        self.mix = old.mix;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let ms = 0.001 * f64::from(self.rate);
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            if fresh {
                let (hz, sync) = lfo_rate(&self.bank, flp::RATE, flp::SYNC, flp::DIVISION);
                self.lfo = self.clock.chunk(cx, &ck, hz, sync);
            }
            let (phase0, inc) = self.lfo;
            let b = &self.bank;
            let base = (b.get(flp::DELAY) * ms) as f32;
            let depth = (b.get(flp::DEPTH) * ms) as f32;
            let fb = (b.get(flp::FEEDBACK) / 100.0) as f32;
            let spread = b.get(flp::SPREAD) / 100.0 * 0.5;
            let mix_to = (b.get(flp::MIX) / 100.0) as f32;
            let mix = self.mix.step(fresh, mix_to);
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let j = ck.idx(i);
                let phase = phase0 + inc * j as f64;
                let mut out = [0.0f32; 2];
                for (ch, o) in out.iter_mut().enumerate() {
                    let p = phase + if ch == 1 { spread } else { 0.0 };
                    *o = self.line.read_linear(base + depth * sine01(p), ch);
                }
                self.line.write([
                    super::delay::protect(f[0] + out[0] * fb),
                    super::delay::protect(f[1] + out[1] * fb),
                ]);
                self.last = out;
                let m = mix.at(j, ck.len);
                f[0] += (out[0] - f[0]) * m;
                f[1] += (out[1] - f[1]) * m;
            }
        }
    }
}

mod php {
    pub const RATE: usize = 0;
    pub const SYNC: usize = 1;
    pub const DIVISION: usize = 2;
    pub const STAGES: usize = 3;
    pub const CENTER: usize = 4;
    pub const DEPTH: usize = 5;
    pub const FEEDBACK: usize = 6;
    pub const SPREAD: usize = 7;
    pub const MIX: usize = 8;
}

const MAX_STAGES: usize = 12;

#[derive(Debug, Clone)]
pub struct Phaser {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    /// First-order all-pass states per channel and stage.
    z: [[f32; MAX_STAGES]; 2],
    coef: [Ramp; 2],
    started: bool,
    last: [f32; 2],
    clock: LfoClock,
    mix: Ramp,
}

impl Phaser {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        Self {
            bank: Bank::new(EffectKind::Phaser, p, rate),
            out_peak: 0.0,
            rate,
            z: [[0.0; MAX_STAGES]; 2],
            coef: [Ramp::hold(0.0); 2],
            started: false,
            last: [0.0; 2],
            clock: LfoClock::default(),
            mix: Ramp::hold((p.get(php::MIX) / 100.0) as f32),
        }
    }

    pub fn adopt(&mut self, old: &mut Phaser) {
        self.bank.adopt(&old.bank);
        self.z = old.z;
        self.coef = old.coef;
        self.started = old.started;
        self.last = old.last;
        self.clock = old.clock;
        self.mix = old.mix;
    }

    fn coefficient(&self, phase: f64) -> f32 {
        let b = &self.bank;
        let octaves = 3.0 * b.get(php::DEPTH) / 100.0;
        let lfo = (TAU * phase).sin();
        let f = (b.get(php::CENTER) * 2f64.powf(octaves * lfo))
            .clamp(20.0, f64::from(self.rate) * 0.45);
        let k = (std::f64::consts::PI * f / f64::from(self.rate)).tan();
        ((k - 1.0) / (k + 1.0)) as f32
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let b = &self.bank;
            let stages = 2 * (b.get(php::STAGES).round() as usize + 1);
            let fb = (b.get(php::FEEDBACK) / 100.0) as f32;
            let spread = b.get(php::SPREAD) / 100.0 * 0.5;
            let mix_to = (b.get(php::MIX) / 100.0) as f32;
            if fresh {
                let (hz, sync) = lfo_rate(&self.bank, php::RATE, php::SYNC, php::DIVISION);
                let (phase0, inc) = self.clock.chunk(cx, &ck, hz, sync);
                let end = phase0 + inc * ck.len as f64;
                let target = [self.coefficient(end), self.coefficient(end + spread)];
                for (c, t) in self.coef.iter_mut().zip(target) {
                    // The first chunk starts on its target (no ramp from zero).
                    if !self.started {
                        *c = Ramp::hold(t);
                    }
                    c.step(true, t);
                }
                self.started = true;
            }
            let coef = self.coef;
            let mix = self.mix.step(fresh, mix_to);
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let j = ck.idx(i);
                for ch in 0..2 {
                    let a = coef[ch].at(j, ck.len);
                    let mut x = f[ch] + self.last[ch] * fb;
                    for s in 0..stages {
                        // y = a·x + z;  z' = x − a·y  (first-order all-pass)
                        let y = a * x + self.z[ch][s];
                        self.z[ch][s] = super::flush(x - a * y);
                        x = y;
                    }
                    self.last[ch] = super::delay::protect(x);
                    let m = mix.at(j, ck.len);
                    f[ch] += (x - f[ch]) * m;
                }
            }
        }
    }
}

mod trp {
    pub const RATE: usize = 0;
    pub const SYNC: usize = 1;
    pub const DIVISION: usize = 2;
    pub const DEPTH: usize = 3;
    pub const SHAPE: usize = 4;
    pub const STEREO: usize = 5;
}

#[derive(Debug, Clone)]
pub struct Tremolo {
    pub bank: Bank,
    pub out_peak: f32,
    clock: LfoClock,
    lfo: (f64, f64),
}

impl Tremolo {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        Self {
            bank: Bank::new(EffectKind::Tremolo, p, rate),
            out_peak: 0.0,
            clock: LfoClock::default(),
            lfo: (0.0, 0.0),
        }
    }

    pub fn adopt(&mut self, old: &mut Tremolo) {
        self.bank.adopt(&old.bank);
        self.clock = old.clock;
        self.lfo = old.lfo;
    }

    /// LFO in 0…1 (1 = full level) for `shape`.
    #[inline]
    fn wave(shape: usize, phase: f64) -> f32 {
        let p = phase.rem_euclid(1.0);
        match shape {
            1 => (1.0 - (2.0 * p - 1.0).abs()) as f32,
            2 => {
                // Square with 5 % raised-cosine edges.
                let edge = 0.05;
                let v = if p < edge {
                    0.5 - 0.5 * (std::f64::consts::PI * p / edge).cos()
                } else if p < 0.5 {
                    1.0
                } else if p < 0.5 + edge {
                    0.5 + 0.5 * (std::f64::consts::PI * (p - 0.5) / edge).cos()
                } else {
                    0.0
                };
                v as f32
            }
            _ => sine01(p),
        }
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            if self.bank.enter(&ck) {
                let (hz, sync) = lfo_rate(&self.bank, trp::RATE, trp::SYNC, trp::DIVISION);
                self.lfo = self.clock.chunk(cx, &ck, hz, sync);
            }
            let (phase0, inc) = self.lfo;
            let b = &self.bank;
            let depth = (b.get(trp::DEPTH) / 100.0) as f32;
            let shape = b.get(trp::SHAPE).round() as usize;
            let offset = b.get(trp::STEREO) / 360.0;
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let p = phase0 + inc * ck.idx(i) as f64;
                f[0] *= 1.0 - depth * (1.0 - Self::wave(shape, p));
                f[1] *= 1.0 - depth * (1.0 - Self::wave(shape, p + offset));
            }
        }
    }
}

mod rop {
    pub const SPEED: usize = 0;
    pub const SLOW: usize = 1;
    pub const FAST: usize = 2;
    pub const ACCEL: usize = 3;
    pub const DRIVE: usize = 4;
    pub const SPREAD: usize = 5;
    pub const MIX: usize = 6;
}

/// Rotary speaker: an 800 Hz Linkwitz–Riley crossover feeds a horn and a
/// drum rotor. Each rotor's angle drives Doppler (a modulated delay) and
/// amplitude modulation for two virtual microphones; rotors accelerate and
/// brake with inertia (the drum is heavier and turns at 85 % of the horn).
#[derive(Debug, Clone)]
pub struct Rotary {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    lp: [Biquad; 2],
    hp: [Biquad; 2],
    horn: Line,
    drum: Line,
    angle: [f64; 2],
    speed: [f64; 2],
    mix: Ramp,
}

impl Rotary {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        let q0 = butterworth_q(1, 0);
        let lp = Biquad::new(Coefs::low_pass(800.0, q0, rate));
        let hp = Biquad::new(Coefs::high_pass(800.0, q0, rate));
        let bank = Bank::new(EffectKind::Rotary, p, rate);
        let start = Self::target_speed(&bank);
        Self {
            bank,
            out_peak: 0.0,
            rate,
            lp: [lp; 2],
            hp: [hp; 2],
            horn: Line::new((0.004 * f64::from(rate)) as usize + 8),
            drum: Line::new((0.004 * f64::from(rate)) as usize + 8),
            angle: [0.0, 0.25],
            speed: [start, start * 0.85],
            mix: Ramp::hold((p.get(rop::MIX) / 100.0) as f32),
        }
    }

    fn target_speed(b: &Bank) -> f64 {
        match b.get(rop::SPEED).round() as usize {
            0 => b.get(rop::SLOW),
            1 => b.get(rop::FAST),
            _ => 0.0,
        }
    }

    pub fn adopt(&mut self, old: &mut Rotary) {
        self.bank.adopt(&old.bank);
        self.lp = old.lp;
        self.hp = old.hp;
        self.horn.adopt(&mut old.horn);
        self.drum.adopt(&mut old.drum);
        self.angle = old.angle;
        self.speed = old.speed;
        self.mix = old.mix;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let rate = f64::from(self.rate);
        let ms = 0.001 * rate;
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let b = &self.bank;
            let target = Self::target_speed(b);
            let accel = b.get(rop::ACCEL);
            let drive = 1.0 + 6.0 * (b.get(rop::DRIVE) / 100.0) as f32;
            let spread = b.get(rop::SPREAD) / 100.0 * 0.25;
            let mix_to = (b.get(rop::MIX) / 100.0) as f32;
            let mix = self.mix.step(fresh, mix_to);
            if fresh {
                // Inertia: the rotors approach their speed once per chunk.
                let len = ck.len as f64;
                self.speed[0] += (target - self.speed[0]) * (1.0 - (-len / (accel * rate)).exp());
                self.speed[1] +=
                    (target * 0.85 - self.speed[1]) * (1.0 - (-len / (accel * 2.5 * rate)).exp());
            }
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let x = 0.5 * (f[0] + f[1]);
                let x = (drive * x).tanh() / drive.tanh();
                let lo = self.lp[0].run(0, f64::from(x));
                let lo = self.lp[1].run(0, lo) as f32;
                let hi = self.hp[0].run(0, f64::from(x));
                let hi = -(self.hp[1].run(0, hi) as f32);
                self.horn.write([hi, hi]);
                self.drum.write([lo, lo]);
                let mut out = [0.0f32; 2];
                for (mic, o) in out.iter_mut().enumerate() {
                    let side = if mic == 0 { -spread } else { spread };
                    let h = TAU * (self.angle[0] + side);
                    let d = TAU * (self.angle[1] + side);
                    let horn = self
                        .horn
                        .read_linear((1.2 + 0.9 * h.cos()) as f32 * ms as f32, 0)
                        * (0.65 + 0.35 * h.sin() as f32);
                    let drum = self
                        .drum
                        .read_linear((1.2 + 0.4 * d.cos()) as f32 * ms as f32, 0)
                        * (0.8 + 0.2 * d.sin() as f32);
                    *o = horn + drum;
                }
                self.angle[0] = (self.angle[0] + self.speed[0] / rate).fract();
                self.angle[1] = (self.angle[1] + self.speed[1] / rate).fract();
                let m = mix.at(ck.idx(i), ck.len);
                f[0] += (out[0] - f[0]) * m;
                f[1] += (out[1] - f[1]) * m;
            }
            for bq in self.lp.iter_mut().chain(self.hp.iter_mut()) {
                bq.flush();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Effect;
    use super::super::testing::*;
    use super::*;

    /// Gain envelope of a tremolo on a constant input: minima once per
    /// cycle, at the musical position (the beat) when synchronised.
    #[test]
    fn synchronised_tremolo_lands_on_the_beat() {
        let rate = 48_000;
        for bpm in [90.0, 120.0] {
            let m = map(bpm);
            let s = slot(EffectKind::Tremolo, &[("division", 9.0), ("depth", 100.0)]);
            let x = vec![1.0f32; 4 * rate as usize * 2];
            let out = run(&mut Effect::new(&s, rate), &x, rate, 100, &m);
            // sine01 is at full level a quarter cycle after the beat.
            let beat = 60.0 / bpm * f64::from(rate);
            for k in 1..4 {
                let peak = ((k as f64 + 0.25) * beat) as usize;
                let trough = ((k as f64 + 0.75) * beat) as usize;
                assert!(out[peak * 2] > 0.999, "{bpm} beat {k}: {}", out[peak * 2]);
                assert!(
                    out[trough * 2] < 0.001,
                    "{bpm} beat {k}: {}",
                    out[trough * 2]
                );
            }
        }
    }

    #[test]
    fn synchronised_lfo_follows_tempo_map_changes() {
        use elyvon_core::tempo::{TempoChange, TempoMap};
        use elyvon_core::{PPQ, Tempo, TimeSignature};
        // 120 BPM for two bars (4 s), then 60 BPM: quarter-note cycles
        // stretch from 0.5 s to 1 s and stay on the beat.
        let rate = 48_000;
        let map = TempoMap::new(
            Tempo::new(120.0).expect("tempo"),
            &[TempoChange {
                tick: u64::from(PPQ) * 8,
                bpm: Tempo::new(60.0).expect("tempo"),
            }],
            TimeSignature::default(),
            &[],
        )
        .expect("map");
        let s = slot(EffectKind::Tremolo, &[("division", 9.0), ("depth", 100.0)]);
        let x = vec![1.0f32; 8 * rate as usize * 2];
        let out = run(&mut Effect::new(&s, rate), &x, rate, 96, &map);
        for k in 0..3 {
            let peak = ((4.0 + k as f64 + 0.25) * f64::from(rate)) as usize;
            let trough = ((4.0 + k as f64 + 0.75) * f64::from(rate)) as usize;
            assert!(out[peak * 2] > 0.999, "beat {k}: {}", out[peak * 2]);
            assert!(out[trough * 2] < 0.001, "beat {k}: {}", out[trough * 2]);
        }
    }

    #[test]
    fn lfo_restarts_identically_from_any_position() {
        // Playing from bar 3 gives the same audio as playing from the start
        // and reaching bar 3 (offline bounce = live playback from a locate).
        let rate = 48_000;
        let m = map(120.0);
        for kind in [
            EffectKind::Chorus,
            EffectKind::Flanger,
            EffectKind::Phaser,
            EffectKind::Tremolo,
        ] {
            let s = slot(kind, &[]);
            let x = vec![0.5f32; 2 * rate as usize * 2];
            let full = run(&mut Effect::new(&s, rate), &x, rate, 128, &m);
            let start = rate as usize; // 1 s
            let mut fx = Effect::new(&s, rate);
            let mut part = x[start * 2..].to_vec();
            let mut pos = start as u64;
            for piece in part.chunks_mut(256) {
                fx.process(piece, &cx(&m, rate, pos));
                pos += (piece.len() / 2) as u64;
            }
            // After the delay lines fill (40 ms) both passes agree.
            let from = (start + 2_400) * 2;
            let worst = full[from..]
                .iter()
                .zip(&part[2_400 * 2..])
                .fold(0f32, |w, (a, b)| w.max((a - b).abs()));
            assert!(worst < 1e-3, "{kind:?}: {worst}");
        }
    }

    #[test]
    fn phaser_notches_move_and_feedback_is_stable() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::Phaser,
            &[("feedback", 95.0), ("depth", 100.0), ("stages", 5.0)],
        );
        let x = sine(700.0, rate, 4 * rate as usize, 1.0);
        let out = run(&mut Effect::new(&s, rate), &x, rate, 64, &m);
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 8.0));
        // Amplitude varies over time as notches sweep past 700 Hz.
        let levels: Vec<f32> = out.chunks(4_800).map(rms).collect();
        let (lo, hi) = levels
            .iter()
            .fold((f32::MAX, 0f32), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(hi / lo.max(1e-6) > 2.0, "{lo} {hi}");
    }

    #[test]
    fn rotary_accelerates_with_inertia() {
        let rate = 48_000;
        let s = slot(EffectKind::Rotary, &[("speed", 0.0), ("accelSeconds", 1.0)]);
        let Effect::Rotary(mut r) = Effect::new(&s, rate) else {
            unreachable!()
        };
        let m = map(120.0);
        r.bank.set(rop::SPEED, 1.0);
        let mut x = vec![0.0f32; rate as usize / 2 * 2];
        r.process(&mut x, &cx(&m, rate, 0));
        // Half a time constant: between slow and fast, closer to slow.
        assert!(
            r.speed[0] > 0.8 + 0.3 && r.speed[0] < 6.7 - 2.0,
            "{}",
            r.speed[0]
        );
        assert!(r.speed[1] < r.speed[0], "the drum is heavier");
    }
}
