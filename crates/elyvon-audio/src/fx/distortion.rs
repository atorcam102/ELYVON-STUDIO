//! Distortion/saturation with optional 2×/4× oversampling, and a
//! bitcrusher.
//!
//! Oversampling uses linear-phase windowed-sinc FIR stages (31 taps at 2×,
//! 33 taps for the second 4× stage), so the latency is an exact number of
//! frames: 15 at 2×, 23 at 4×. It is reported to the mixer's delay
//! compensation and the dry path is delayed to match. The setting is fixed
//! per instance (changing it rebuilds the effect).

use elyvon_project::effects::{EffectKind, ParamValues};

use super::delay::Line;
use super::filters::{Biquad, Coefs};
use super::utility::Rng;
use super::{Bank, FxContext, Ramp, db};

const TAPS_2X: usize = 31;
const TAPS_4X: usize = 33;

mod dsp {
    pub const CURVE: usize = 0;
    pub const DRIVE: usize = 1;
    pub const TONE: usize = 2;
    pub const OUTPUT: usize = 3;
    pub const MIX: usize = 4;
    pub const OVERSAMPLING: usize = 5;
}

/// Latency in frames for the oversampling setting stored in `p`.
pub fn latency_for(p: &ParamValues) -> usize {
    match p.get(dsp::OVERSAMPLING).round() as usize {
        0 => 0,
        1 => (TAPS_2X - 1) / 2,
        _ => (TAPS_2X - 1) / 2 + (TAPS_4X - 1) / 4,
    }
}

/// Low-pass FIR with its cutoff at `cutoff` (fraction of the sample rate),
/// Blackman window, unity DC gain.
fn design(taps: usize, cutoff: f64) -> Vec<f32> {
    let m = (taps - 1) as f64 / 2.0;
    let mut h: Vec<f64> = (0..taps)
        .map(|k| {
            let x = k as f64 - m;
            let sinc = if x == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
            };
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * k as f64 / (taps - 1) as f64).cos()
                + 0.08 * (4.0 * std::f64::consts::PI * k as f64 / (taps - 1) as f64).cos();
            sinc * w
        })
        .collect();
    let sum: f64 = h.iter().sum();
    for v in &mut h {
        *v /= sum;
    }
    h.into_iter().map(|v| v as f32).collect()
}

/// A FIR with a circular history per channel.
#[derive(Debug, Clone)]
struct Fir {
    h: Vec<f32>,
    hist: [Vec<f32>; 2],
    at: usize,
}

impl Fir {
    fn new(h: Vec<f32>) -> Self {
        let n = h.len();
        Self {
            h,
            hist: [vec![0.0; n], vec![0.0; n]],
            at: 0,
        }
    }

    #[inline]
    fn push(&mut self, l: f32, r: f32) {
        self.at = if self.at == 0 {
            self.h.len() - 1
        } else {
            self.at - 1
        };
        self.hist[0][self.at] = l;
        self.hist[1][self.at] = r;
    }

    #[inline]
    fn output(&self, ch: usize) -> f32 {
        let n = self.h.len();
        let hist = &self.hist[ch];
        let (a, b) = hist.split_at(self.at);
        // hist[at..] are the newest samples, then hist[..at].
        let mut y = 0.0f32;
        for (h, x) in self.h[..n - self.at].iter().zip(b) {
            y += h * x;
        }
        for (h, x) in self.h[n - self.at..].iter().zip(a) {
            y += h * x;
        }
        y
    }
}

/// One 2× stage: up-sampling (zero stuffing + interpolation filter, gain 2)
/// and decimation filter.
#[derive(Debug, Clone)]
struct Stage {
    up: Fir,
    down: Fir,
}

impl Stage {
    fn new(taps: usize) -> Self {
        let h = design(taps, 0.23);
        Self {
            up: Fir::new(h.clone()),
            down: Fir::new(h),
        }
    }

    /// Two high-rate frames for one input frame.
    #[inline]
    fn up(&mut self, l: f32, r: f32) -> [[f32; 2]; 2] {
        self.up.push(2.0 * l, 2.0 * r);
        let a = [self.up.output(0), self.up.output(1)];
        self.up.push(0.0, 0.0);
        let b = [self.up.output(0), self.up.output(1)];
        [a, b]
    }

    /// One output frame for two high-rate frames.
    #[inline]
    fn down(&mut self, a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
        self.down.push(a[0], a[1]);
        let y = [self.down.output(0), self.down.output(1)];
        self.down.push(b[0], b[1]);
        y
    }
}

#[inline]
fn shape(curve: usize, x: f32) -> f32 {
    match curve {
        1 => x.clamp(-1.0, 1.0),
        2 => {
            // Asymmetric "tube": softer on the negative half-wave.
            if x >= 0.0 {
                x.tanh()
            } else {
                (0.6 * x).tanh() / 0.6
            }
        }
        3 => {
            // Foldback: reflects at ±1 (bounded for any drive).
            let t = (x + 1.0).rem_euclid(4.0);
            if t < 2.0 { t - 1.0 } else { 3.0 - t }
        }
        _ => x.tanh(),
    }
}

#[derive(Debug, Clone)]
pub struct Distortion {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    factor: usize,
    stages: Vec<Stage>,
    latency: usize,
    dry: Line,
    tone: Biquad,
    tone_freq: f64,
    dc: [(f32, f32); 2],
    dc_r: f32,
    gains: [Ramp; 3],
}

impl Distortion {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        let os = p.get(dsp::OVERSAMPLING).round() as usize;
        let stages = match os {
            0 => Vec::new(),
            1 => vec![Stage::new(TAPS_2X)],
            _ => vec![Stage::new(TAPS_2X), Stage::new(TAPS_4X)],
        };
        let latency = latency_for(p);
        let mut d = Self {
            bank: Bank::new(EffectKind::Distortion, p, rate),
            out_peak: 0.0,
            rate,
            factor: 1 << stages.len(),
            stages,
            latency,
            dry: Line::new(latency + 8),
            tone: Biquad::default(),
            tone_freq: 0.0,
            dc: [(0.0, 0.0); 2],
            dc_r: (1.0 - 2.0 * std::f64::consts::PI * 10.0 / f64::from(rate)) as f32,
            gains: [Ramp::hold(0.0); 3],
        };
        d.gains = d.levels().map(Ramp::hold);
        d
    }

    fn levels(&self) -> [f32; 3] {
        let b = &self.bank;
        [
            db(b.get(dsp::DRIVE)) as f32,
            db(b.get(dsp::OUTPUT)) as f32,
            (b.get(dsp::MIX) / 100.0) as f32,
        ]
    }

    pub fn latency(&self) -> usize {
        self.latency
    }

    pub fn cost(&self) -> f32 {
        2.0 + 3.0 * self.factor as f32 * self.stages.len().max(1) as f32
    }

    pub fn adopt(&mut self, old: &mut Distortion) {
        self.bank.adopt(&old.bank);
        if self.factor == old.factor {
            std::mem::swap(&mut self.stages, &mut old.stages);
            self.dry.adopt(&mut old.dry);
        }
        self.tone.set_state(old.tone.state());
        self.dc = old.dc;
        self.gains = old.gains;
    }

    #[inline]
    fn saturate(&mut self, curve: usize, l: f32, r: f32) -> [f32; 2] {
        match self.stages.len() {
            0 => [shape(curve, l), shape(curve, r)],
            1 => {
                let [a, b] = self.stages[0].up(l, r);
                let a = [shape(curve, a[0]), shape(curve, a[1])];
                let b = [shape(curve, b[0]), shape(curve, b[1])];
                self.stages[0].down(a, b)
            }
            _ => {
                let [a, b] = self.stages[0].up(l, r);
                let mut mid = [[0.0f32; 2]; 2];
                for (m, x) in mid.iter_mut().zip([a, b]) {
                    let [p, q] = self.stages[1].up(x[0], x[1]);
                    let p = [shape(curve, p[0]), shape(curve, p[1])];
                    let q = [shape(curve, q[0]), shape(curve, q[1])];
                    *m = self.stages[1].down(p, q);
                }
                self.stages[0].down(mid[0], mid[1])
            }
        }
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let curve = self.bank.get(dsp::CURVE).round() as usize;
            let tone = self.bank.get(dsp::TONE);
            if tone != self.tone_freq {
                self.tone.c = if tone < 19_900.0 {
                    Coefs::low_pass(tone, std::f64::consts::FRAC_1_SQRT_2, self.rate)
                } else {
                    Coefs::IDENTITY
                };
                self.tone_freq = tone;
            }
            let to = self.levels();
            let r: [Ramp; 3] = std::array::from_fn(|i| self.gains[i].step(fresh, to[i]));
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let j = ck.idx(i);
                let (drive, out, mix) =
                    (r[0].at(j, ck.len), r[1].at(j, ck.len), r[2].at(j, ck.len));
                let dry = if self.latency == 0 {
                    [f[0], f[1]]
                } else {
                    let d = [
                        self.dry.read(self.latency as f32, 0),
                        self.dry.read(self.latency as f32, 1),
                    ];
                    self.dry.write([f[0], f[1]]);
                    d
                };
                let y = self.saturate(curve, f[0] * drive, f[1] * drive);
                for ch in 0..2 {
                    let t = self.tone.run(ch, f64::from(y[ch])) as f32;
                    let (x1, y1) = self.dc[ch];
                    let o = t - x1 + self.dc_r * y1;
                    self.dc[ch] = (t, super::flush(o));
                    f[ch] = dry[ch] + (o * out - dry[ch]) * mix;
                }
            }
            self.tone.flush();
        }
    }
}

mod bcp {
    pub const BITS: usize = 0;
    pub const DOWNSAMPLE: usize = 1;
    pub const DRIVE: usize = 2;
    pub const DITHER: usize = 3;
    pub const MIX: usize = 4;
}

#[derive(Debug, Clone)]
pub struct Bitcrusher {
    pub bank: Bank,
    pub out_peak: f32,
    hold: [f32; 2],
    count: f64,
    rng: Rng,
    mix: Ramp,
}

impl Bitcrusher {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        Self {
            bank: Bank::new(EffectKind::Bitcrusher, p, rate),
            out_peak: 0.0,
            hold: [0.0; 2],
            count: f64::MAX,
            rng: Rng::new(0x5eed_1e55),
            mix: Ramp::hold((p.get(bcp::MIX) / 100.0) as f32),
        }
    }

    pub fn adopt(&mut self, old: &mut Bitcrusher) {
        self.bank.adopt(&old.bank);
        self.hold = old.hold;
        self.count = old.count;
        self.rng = old.rng;
        self.mix = old.mix;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let b = &self.bank;
            let step = (2.0 / 2f64.powf(b.get(bcp::BITS))) as f32;
            let factor = b.get(bcp::DOWNSAMPLE);
            let drive = db(b.get(bcp::DRIVE)) as f32;
            let dither = b.get(bcp::DITHER) >= 0.5;
            let mix_to = (b.get(bcp::MIX) / 100.0) as f32;
            let mix = self.mix.step(fresh, mix_to);
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                self.count += 1.0;
                if self.count >= factor {
                    // The first frame after construction always samples.
                    self.count = if self.count > 2.0 * factor {
                        0.0
                    } else {
                        self.count - factor
                    };
                    for ch in 0..2 {
                        let mut x = (f[ch] * drive).clamp(-1.0, 1.0);
                        if dither {
                            x += (self.rng.next() + self.rng.next()) * 0.5 * step;
                        }
                        self.hold[ch] = (x / step).round() * step;
                    }
                }
                let m = mix.at(ck.idx(i), ck.len);
                for ch in 0..2 {
                    f[ch] = if m >= 1.0 {
                        self.hold[ch]
                    } else {
                        f[ch] + (self.hold[ch] - f[ch]) * m
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Effect;
    use super::super::testing::*;
    use super::*;

    fn goertzel(x: &[f32], freq: f64, rate: u32) -> f64 {
        let w = 2.0 * std::f64::consts::PI * freq / f64::from(rate);
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for v in x.chunks(2) {
            let s = f64::from(v[0]) + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - c * s1 * s2).sqrt() / (x.len() / 2) as f64
    }

    #[test]
    fn reported_latency_is_the_measured_delay() {
        let rate = 48_000;
        let m = map(120.0);
        for (os, expected) in [(0.0, 0usize), (1.0, 15), (2.0, 23)] {
            let s = slot(
                EffectKind::Distortion,
                &[
                    ("oversampling", os),
                    ("driveDb", 0.0),
                    ("outputDb", 0.0),
                    ("toneHz", 20_000.0),
                    ("mix", 100.0),
                ],
            );
            let mut fx = Effect::new(&s, rate);
            assert_eq!(fx.latency(), expected);
            let mut x = vec![0.0f32; 512 * 2];
            x[100 * 2] = 0.01;
            let out = run(&mut fx, &x, rate, 64, &m);
            let peak = out
                .chunks(2)
                .enumerate()
                .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
                .map_or(0, |(i, _)| i);
            assert_eq!(peak, 100 + expected, "oversampling {os}");
            // The dry path is aligned too: at 50 % mix there is one peak.
            let s = slot(
                EffectKind::Distortion,
                &[
                    ("oversampling", os),
                    ("driveDb", 0.0),
                    ("outputDb", 0.0),
                    ("toneHz", 20_000.0),
                    ("mix", 50.0),
                ],
            );
            let out = run(&mut Effect::new(&s, rate), &x, rate, 64, &m);
            assert!(out[(100 + expected) * 2] > 0.009, "{os}");
        }
    }

    #[test]
    fn oversampling_reduces_aliasing() {
        let rate = 48_000;
        let m = map(120.0);
        let x = sine(7_000.0, rate, rate as usize, 0.8);
        // Hard clipping of 7 kHz: the 7th harmonic (49 kHz) aliases to 1 kHz.
        let alias = |os: f64| {
            let s = slot(
                EffectKind::Distortion,
                &[
                    ("oversampling", os),
                    ("curve", 1.0),
                    ("driveDb", 18.0),
                    ("toneHz", 20_000.0),
                ],
            );
            let out = run(&mut Effect::new(&s, rate), &x, rate, 128, &m);
            goertzel(&out[24_000..], 1_000.0, rate)
        };
        let (a1, a4) = (alias(0.0), alias(2.0));
        assert!(a4 < a1 * 0.3, "1×: {a1}, 4×: {a4}");
    }

    #[test]
    fn bitcrusher_quantises_to_its_resolution() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::Bitcrusher,
            &[("bits", 3.0), ("downsample", 1.0)],
        );
        let x = sine(100.0, rate, 4_800, 0.9);
        let out = run(&mut Effect::new(&s, rate), &x, rate, 64, &m);
        let step = 2.0 / 8.0;
        for v in &out {
            let q = v / step;
            assert!((q - q.round()).abs() < 1e-5, "{v}");
        }
        // Downsampling by 4 holds each value for 4 frames.
        let s = slot(
            EffectKind::Bitcrusher,
            &[("bits", 24.0), ("downsample", 4.0)],
        );
        let out = run(&mut Effect::new(&s, rate), &x, rate, 64, &m);
        for k in 0..100 {
            let f = &out[k * 8..k * 8 + 8];
            assert!(f.chunks(2).all(|p| p[0] == f[0]), "{k}");
        }
    }

    #[test]
    fn foldback_is_bounded_for_any_drive() {
        for x in [-100.0f32, -3.3, -1.0, 0.0, 0.7, 2.5, 47.0] {
            assert!(shape(3, x).abs() <= 1.0 + 1e-6);
        }
    }
}
