//! Utilities: gain/balance, stereo width with a bass mono-maker, DC
//! blocker, polarity and channel swap; and a deterministic test generator
//! (white and pink noise, sine, logarithmic sweep).

use elyvon_project::effects::{EffectKind, ParamValues};

use super::filters::{Biquad, Coefs};
use super::{Bank, CONTROL, FxContext, Ramp, db};

/// A switch that crossfades over 5 ms instead of stepping.
#[derive(Debug, Clone, Copy)]
pub struct Fade {
    pub ramp: Ramp,
    step: f32,
}

impl Fade {
    pub fn new(on: bool, rate: u32) -> Self {
        Self {
            ramp: Ramp::hold(if on { 1.0 } else { 0.0 }),
            step: (CONTROL as f64 / (0.005 * f64::from(rate.max(1)))).min(1.0) as f32,
        }
    }

    /// On a new control chunk, moves towards `on` by one step. Returns the
    /// ramp of the current chunk.
    pub fn next(&mut self, fresh: bool, on: bool) -> Ramp {
        let v = self.ramp.to;
        let to = if on {
            (v + self.step).min(1.0)
        } else {
            (v - self.step).max(0.0)
        };
        self.ramp.step(fresh, to)
    }
}

mod sp {
    pub const GAIN: usize = 0;
    pub const BALANCE: usize = 1;
    pub const WIDTH: usize = 2;
    pub const MONO_BELOW: usize = 3;
    pub const DC: usize = 4;
    pub const INV_L: usize = 5;
    pub const INV_R: usize = 6;
    pub const SWAP: usize = 7;
}

#[derive(Debug, Clone)]
pub struct StereoTool {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    gain: [Ramp; 2],
    width: Ramp,
    /// Linkwitz–Riley (2 × Butterworth) high-pass on the side signal.
    side: [Biquad; 2],
    side_freq: f64,
    mono: Fade,
    dc: Fade,
    dc_state: [(f32, f32); 2],
    dc_r: f32,
    inv: [Fade; 2],
    swap: Fade,
}

impl StereoTool {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        let mut s = Self {
            bank: Bank::new(EffectKind::StereoTool, p, rate),
            out_peak: 0.0,
            rate,
            gain: [Ramp::hold(1.0); 2],
            width: Ramp::hold((p.get(sp::WIDTH) / 100.0) as f32),
            side: [Biquad::default(); 2],
            side_freq: 0.0,
            mono: Fade::new(p.get(sp::MONO_BELOW) > 20.5, rate),
            dc: Fade::new(p.get(sp::DC) >= 0.5, rate),
            dc_state: [(0.0, 0.0); 2],
            dc_r: (1.0 - 2.0 * std::f64::consts::PI * 8.0 / f64::from(rate.max(1))) as f32,
            inv: [
                Fade::new(p.get(sp::INV_L) >= 0.5, rate),
                Fade::new(p.get(sp::INV_R) >= 0.5, rate),
            ],
            swap: Fade::new(p.get(sp::SWAP) >= 0.5, rate),
        };
        let g = s.gains();
        s.gain = [Ramp::hold(g[0]), Ramp::hold(g[1])];
        s
    }

    fn gains(&self) -> [f32; 2] {
        let g = db(self.bank.get(sp::GAIN));
        let bal = self.bank.get(sp::BALANCE) / 100.0;
        [
            (g * (1.0 - bal).min(1.0)) as f32,
            (g * (1.0 + bal).min(1.0)) as f32,
        ]
    }

    pub fn adopt(&mut self, old: &mut StereoTool) {
        self.bank.adopt(&old.bank);
        self.gain = old.gain;
        self.width = old.width;
        for (a, b) in self.side.iter_mut().zip(&old.side) {
            a.set_state(b.state());
        }
        self.mono.ramp = old.mono.ramp;
        self.dc.ramp = old.dc.ramp;
        self.dc_state = old.dc_state;
        self.inv[0].ramp = old.inv[0].ramp;
        self.inv[1].ramp = old.inv[1].ramp;
        self.swap.ramp = old.swap.ramp;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let b = &self.bank;
            let f = b.get(sp::MONO_BELOW);
            if f > 20.5 && f != self.side_freq {
                let c = Coefs::high_pass(f, std::f64::consts::FRAC_1_SQRT_2, self.rate);
                self.side[0].c = c;
                self.side[1].c = c;
                self.side_freq = f;
            }
            let (dc_on, inv_l, inv_r, swap_on) = (
                b.get(sp::DC) >= 0.5,
                b.get(sp::INV_L) >= 0.5,
                b.get(sp::INV_R) >= 0.5,
                b.get(sp::SWAP) >= 0.5,
            );
            let width_to = (b.get(sp::WIDTH) / 100.0) as f32;
            let mono = self.mono.next(fresh, f > 20.5);
            let dc = self.dc.next(fresh, dc_on);
            let inv = [
                self.inv[0].next(fresh, inv_l),
                self.inv[1].next(fresh, inv_r),
            ];
            let swap = self.swap.next(fresh, swap_on);
            let width = self.width.step(fresh, width_to);
            let target = self.gains();
            let gl = self.gain[0].step(fresh, target[0]);
            let gr = self.gain[1].step(fresh, target[1]);
            let len = ck.len;
            let ms_needed =
                width.from != 1.0 || width.to != 1.0 || mono.from > 0.0 || mono.to > 0.0;
            let dc_active = dc.from > 0.0 || dc.to > 0.0;
            for (i, fr) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let i = ck.idx(i);
                let (mut l, mut r) = (fr[0], fr[1]);
                if dc_active {
                    let k = dc.at(i, len);
                    for (ch, x) in [&mut l, &mut r].into_iter().enumerate() {
                        let (x1, y1) = self.dc_state[ch];
                        let y = *x - x1 + self.dc_r * y1;
                        self.dc_state[ch] = (*x, super::flush(y));
                        *x += (y - *x) * k;
                    }
                }
                l *= 1.0 - 2.0 * inv[0].at(i, len);
                r *= 1.0 - 2.0 * inv[1].at(i, len);
                let k = swap.at(i, len);
                if k > 0.0 {
                    let (a, b) = (l, r);
                    l = a + (b - a) * k;
                    r = b + (a - b) * k;
                }
                if ms_needed {
                    let m = 0.5 * (l + r);
                    let mut s = 0.5 * (l - r);
                    let mk = mono.at(i, len);
                    if mk > 0.0 {
                        let hp = self.side[0].run(0, f64::from(s));
                        let hp = self.side[1].run(0, hp) as f32;
                        s += (hp - s) * mk;
                    }
                    s *= width.at(i, len);
                    l = m + s;
                    r = m - s;
                }
                fr[0] = l * gl.at(i, len);
                fr[1] = r * gr.at(i, len);
            }
            self.side[0].flush();
            self.side[1].flush();
        }
    }
}

mod gn {
    pub const SIGNAL: usize = 0;
    pub const LEVEL: usize = 1;
    pub const FREQ: usize = 2;
    pub const MODE: usize = 3;
    pub const SWEEP: usize = 4;
}

/// Small deterministic generator (xorshift32): reproducible test signals.
#[derive(Debug, Clone, Copy)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    /// Uniform in [−1, 1).
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / 2_147_483_648.0) - 1.0
    }
}

/// Pink-noise filter (Paul Kellet's published "refined" coefficients) on
/// one channel.
#[derive(Debug, Clone, Copy, Default)]
struct Pink {
    b: [f32; 7],
}

impl Pink {
    #[inline]
    fn next(&mut self, w: f32) -> f32 {
        let b = &mut self.b;
        b[0] = 0.998_86 * b[0] + w * 0.055_517_9;
        b[1] = 0.993_32 * b[1] + w * 0.075_075_9;
        b[2] = 0.969_00 * b[2] + w * 0.153_852;
        b[3] = 0.866_50 * b[3] + w * 0.310_485_6;
        b[4] = 0.550_00 * b[4] + w * 0.532_952_2;
        b[5] = -0.761_6 * b[5] - w * 0.016_898_0;
        let out = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.536_2;
        b[6] = w * 0.115_926;
        out
    }
}

#[derive(Debug, Clone)]
pub struct Generator {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    rng: [Rng; 2],
    pink: [Pink; 2],
    /// Normalises the pink filter to unit RMS for unit-RMS white input.
    pink_norm: f32,
    phase: f64,
    sweep_t: f64,
    level: Ramp,
}

impl Generator {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        // Exact output variance of the pink filter: energy of its impulse
        // response (white noise has unit variance after the √3 scale).
        let mut probe = Pink::default();
        let mut energy = 0.0f64;
        for i in 0..200_000 {
            let y = f64::from(probe.next(if i == 0 { 1.0 } else { 0.0 }));
            energy += y * y;
        }
        Self {
            bank: Bank::new(EffectKind::TestGenerator, p, rate),
            out_peak: 0.0,
            rate,
            rng: [Rng::new(0x1234_5678), Rng::new(0x0bad_cafe)],
            pink: [Pink::default(); 2],
            pink_norm: (1.0 / energy.sqrt()) as f32,
            phase: 0.0,
            sweep_t: 0.0,
            level: Ramp::hold(db(p.get(gn::LEVEL)) as f32),
        }
    }

    pub fn adopt(&mut self, old: &mut Generator) {
        self.bank.adopt(&old.bank);
        self.rng = old.rng;
        self.pink = old.pink;
        self.phase = old.phase;
        self.sweep_t = old.sweep_t;
        self.level = old.level;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let rate = f64::from(self.rate);
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let b = &self.bank;
            let signal = b.get(gn::SIGNAL).round() as usize;
            let add = b.get(gn::MODE) >= 0.5;
            let freq = b.get(gn::FREQ);
            let sweep = b.get(gn::SWEEP);
            let to = db(b.get(gn::LEVEL)) as f32;
            let level = self.level.step(fresh, to);
            let sqrt3 = 3f32.sqrt();
            for (i, fr) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let g = level.at(ck.idx(i), ck.len);
                let v = match signal {
                    0 => [self.rng[0].next() * sqrt3, self.rng[1].next() * sqrt3],
                    1 => [
                        self.pink[0].next(self.rng[0].next() * sqrt3) * self.pink_norm,
                        self.pink[1].next(self.rng[1].next() * sqrt3) * self.pink_norm,
                    ],
                    _ => {
                        let f = if signal == 2 {
                            freq
                        } else {
                            let t = self.sweep_t / sweep.max(0.1);
                            self.sweep_t += 1.0 / rate;
                            if self.sweep_t >= sweep {
                                self.sweep_t = 0.0;
                            }
                            20.0 * 1_000f64.powf(t.min(1.0))
                        };
                        self.phase = (self.phase + f / rate).fract();
                        let s = (2.0 * std::f64::consts::PI * self.phase).sin() as f32;
                        [s, s]
                    }
                };
                if add {
                    fr[0] += v[0] * g;
                    fr[1] += v[1] * g;
                } else {
                    fr[0] = v[0] * g;
                    fr[1] = v[1] * g;
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

    #[test]
    fn width_zero_is_mono_and_two_hundred_doubles_the_side() {
        let rate = 48_000;
        let m = map(120.0);
        let x: Vec<f32> = (0..4_800)
            .flat_map(|i| {
                [
                    0.5 * (i as f32 * 0.01).sin(),
                    0.2 * (i as f32 * 0.013).cos(),
                ]
            })
            .collect();
        let out = run(
            &mut Effect::new(&slot(EffectKind::StereoTool, &[("width", 0.0)]), rate),
            &x,
            rate,
            64,
            &m,
        );
        for f in out.chunks(2).skip(200) {
            assert!((f[0] - f[1]).abs() < 1e-6);
        }
        let out = run(
            &mut Effect::new(&slot(EffectKind::StereoTool, &[("width", 200.0)]), rate),
            &x,
            rate,
            64,
            &m,
        );
        for (o, i) in out.chunks(2).zip(x.chunks(2)).skip(200) {
            assert!(((o[0] - o[1]) - 2.0 * (i[0] - i[1])).abs() < 1e-5);
            assert!(((o[0] + o[1]) - (i[0] + i[1])).abs() < 1e-5);
        }
    }

    #[test]
    fn mono_maker_removes_the_bass_side_only() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(EffectKind::StereoTool, &[("monoBelowHz", 150.0)]);
        // Anti-phase 50 Hz (pure side) is removed; anti-phase 2 kHz stays.
        for (freq, keep) in [(50.0, false), (2_000.0, true)] {
            let x: Vec<f32> = sine(freq, rate, 48_000, 0.5)
                .chunks(2)
                .flat_map(|f| [f[0], -f[1]])
                .collect();
            let out = run(&mut Effect::new(&s, rate), &x, rate, 128, &m);
            let side: Vec<f32> = out.chunks(2).skip(24_000).map(|f| f[0] - f[1]).collect();
            let r = rms(&side) / rms(&x[48_000..]) / 2.0;
            if keep {
                assert!(r > 0.97, "{freq}: {r}");
            } else {
                assert!(r < 0.12, "{freq}: {r}");
            }
        }
    }

    #[test]
    fn dc_blocker_and_polarity() {
        let rate = 48_000;
        let m = map(120.0);
        let x = vec![0.25f32; 48_000 * 2];
        let out = run(
            &mut Effect::new(&slot(EffectKind::StereoTool, &[("dcBlock", 1.0)]), rate),
            &x,
            rate,
            128,
            &m,
        );
        assert!(out[out.len() - 2].abs() < 1e-3, "{}", out[out.len() - 2]);
        let out = run(
            &mut Effect::new(&slot(EffectKind::StereoTool, &[("invertLeft", 1.0)]), rate),
            &x,
            rate,
            128,
            &m,
        );
        assert_eq!((out[0], out[1]), (-0.25, 0.25));
    }

    #[test]
    fn generator_levels_are_calibrated_and_reproducible() {
        let rate = 48_000;
        let m = map(120.0);
        let silence = vec![0.0f32; 96_000 * 2];
        for (signal, expected_rms) in [
            (0.0, db(-18.0)),
            (1.0, db(-18.0)),
            (2.0, db(-18.0) / 2f64.sqrt()),
        ] {
            let s = slot(EffectKind::TestGenerator, &[("signal", signal)]);
            let a = run(&mut Effect::new(&s, rate), &silence, rate, 128, &m);
            let b = run(&mut Effect::new(&s, rate), &silence, rate, 64, &m);
            assert_eq!(a, b, "deterministic");
            let r = f64::from(rms(&a));
            assert!(
                (20.0 * (r / expected_rms).log10()).abs() < 0.5,
                "signal {signal}: {r}"
            );
        }
    }
}
