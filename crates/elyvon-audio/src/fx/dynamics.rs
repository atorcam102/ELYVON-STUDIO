//! Compressor and gate/expander.
//!
//! Both are feed-forward designs with a log-domain gain computer: the
//! detector (the signal itself or an external key) is filtered, its level
//! measured in dB, the static curve gives the wanted gain change and an
//! attack/release smoother shapes it in time (decoupled branching smoother,
//! as described by Giannoulis, Massberg and Reiss, JAES 2012). No look-ahead
//! is used: no latency is added. The largest gain reduction since the last
//! read feeds the mixer's GR meter.

use elyvon_project::effects::{EffectKind, ParamValues};

use super::filters::{Biquad, Coefs};
use super::{Bank, FxContext, Ramp, db, time_coef};

/// Static curve of a downward compressor with a soft knee: output level for
/// input level `x` (all dB).
#[inline]
pub fn compressor_curve(x: f64, threshold: f64, ratio: f64, knee: f64) -> f64 {
    let over = x - threshold;
    if knee > 0.0 && 2.0 * over.abs() <= knee {
        let k = over + knee / 2.0;
        x + (1.0 / ratio - 1.0) * k * k / (2.0 * knee)
    } else if over > 0.0 {
        threshold + over / ratio
    } else {
        x
    }
}

#[inline]
fn level_db(x: f64) -> f64 {
    20.0 * x.max(1e-10).log10()
}

/// The key signal for this chunk: the external key when enabled and routed.
fn key<'a>(cx: &FxContext<'a>, external: bool, off: usize, n: usize) -> Option<&'a [f32]> {
    if !external {
        return None;
    }
    cx.key.and_then(|k| k.get(off * 2..(off + n) * 2))
}

#[derive(Debug, Clone)]
pub struct Compressor {
    pub bank: Bank,
    pub out_peak: f32,
    /// Largest gain reduction (dB) since the meter last read it.
    pub reduction: f32,
    rate: u32,
    sc: Biquad,
    sc_freq: f64,
    /// Mean square of the detector per channel (RMS mode).
    ms: [f64; 2],
    /// Smoothed gain reduction per channel, dB (≥ 0).
    env: [f64; 2],
    mix: Ramp,
}

mod cp {
    pub const THRESHOLD: usize = 0;
    pub const RATIO: usize = 1;
    pub const ATTACK: usize = 2;
    pub const RELEASE: usize = 3;
    pub const KNEE: usize = 4;
    pub const MAKEUP: usize = 5;
    pub const AUTO: usize = 6;
    pub const DETECTOR: usize = 7;
    pub const SC_HPF: usize = 8;
    pub const EXTERNAL: usize = 9;
    pub const MIX: usize = 10;
    pub const LINK: usize = 11;
}

impl Compressor {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        Self {
            bank: Bank::new(EffectKind::Compressor, p, rate),
            out_peak: 0.0,
            reduction: 0.0,
            rate,
            sc: Biquad::default(),
            sc_freq: 0.0,
            ms: [0.0; 2],
            env: [0.0; 2],
            mix: Ramp::hold((p.get(cp::MIX) / 100.0) as f32),
        }
    }

    pub fn adopt(&mut self, old: &mut Compressor) {
        self.bank.adopt(&old.bank);
        self.sc.set_state(old.sc.state());
        self.ms = old.ms;
        self.env = old.env;
        self.mix = old.mix;
    }

    /// Make-up gain (dB) including the automatic part: half the reduction a
    /// full-scale signal would get, so the perceived level stays similar.
    fn makeup(&self) -> f64 {
        let b = &self.bank;
        let mut m = b.get(cp::MAKEUP);
        if b.get(cp::AUTO) >= 0.5 {
            let t = b.get(cp::THRESHOLD);
            m += 0.5 * (t - compressor_curve(0.0, t, b.get(cp::RATIO), b.get(cp::KNEE))).max(0.0);
        }
        m
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let rms_coef = time_coef(10.0, self.rate);
        for c in cx.chunks(frames) {
            let (off, n) = (c.off, c.n);
            let fresh = self.bank.enter(&c);
            let b = &self.bank;
            let (t, r, k) = (b.get(cp::THRESHOLD), b.get(cp::RATIO), b.get(cp::KNEE));
            let att = time_coef(b.get(cp::ATTACK), self.rate);
            let rel = time_coef(b.get(cp::RELEASE), self.rate);
            let rms = b.get(cp::DETECTOR) >= 0.5;
            let link = b.get(cp::LINK) >= 0.5;
            let hpf = b.get(cp::SC_HPF);
            if hpf != self.sc_freq {
                self.sc.c = if hpf > 20.5 {
                    Coefs::high_pass(hpf, std::f64::consts::FRAC_1_SQRT_2, self.rate)
                } else {
                    Coefs::IDENTITY
                };
                self.sc_freq = hpf;
            }
            let makeup = self.makeup();
            let external = b.get(cp::EXTERNAL) >= 0.5;
            let mix = self
                .mix
                .step(fresh, (self.bank.get(cp::MIX) / 100.0) as f32);
            let key = key(cx, external, off, n);
            let piece = &mut buf[off * 2..(off + n) * 2];
            let mut worst = 0.0f64;
            for (i, f) in piece.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let src = key.map_or([f[0], f[1]], |k| [k[i * 2], k[i * 2 + 1]]);
                let mut level = [0.0f64; 2];
                for ch in 0..2 {
                    let d = self.sc.run(ch, f64::from(src[ch]));
                    level[ch] = if rms {
                        self.ms[ch] = d * d + (self.ms[ch] - d * d) * rms_coef;
                        self.ms[ch].sqrt()
                    } else {
                        d.abs()
                    };
                }
                if link {
                    let m = level[0].max(level[1]);
                    level = [m, m];
                }
                let w = mix.at(c.idx(i), c.len);
                for ch in 0..2 {
                    let x = level_db(level[ch]);
                    let gr = x - compressor_curve(x, t, r, k);
                    let e = self.env[ch];
                    let coef = if gr > e { att } else { rel };
                    self.env[ch] = gr + (e - gr) * coef;
                    worst = worst.max(self.env[ch]);
                    let g = db(makeup - self.env[ch]) as f32;
                    let dry = f[ch];
                    f[ch] = dry + (dry * g - dry) * w;
                }
            }
            self.sc.flush();
            for v in &mut self.env {
                *v = super::flush64(*v);
            }
            self.reduction = self.reduction.max(worst as f32);
        }
    }
}

mod gp {
    pub const MODE: usize = 0;
    pub const THRESHOLD: usize = 1;
    pub const RANGE: usize = 2;
    pub const RATIO: usize = 3;
    pub const ATTACK: usize = 4;
    pub const HOLD: usize = 5;
    pub const RELEASE: usize = 6;
    pub const HYSTERESIS: usize = 7;
    pub const SC_HPF: usize = 8;
    pub const SC_LPF: usize = 9;
    pub const EXTERNAL: usize = 10;
}

#[derive(Debug, Clone)]
pub struct Gate {
    pub bank: Bank,
    pub out_peak: f32,
    pub reduction: f32,
    rate: u32,
    hpf: Biquad,
    lpf: Biquad,
    filters: (f64, f64),
    /// Peak follower of the detector (linear).
    follow: f64,
    open: bool,
    hold_left: u64,
    /// Current gain, dB (≤ 0).
    gain_db: f64,
}

impl Gate {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        Self {
            bank: Bank::new(EffectKind::Gate, p, rate),
            out_peak: 0.0,
            reduction: 0.0,
            rate,
            hpf: Biquad::default(),
            lpf: Biquad::default(),
            filters: (0.0, 0.0),
            follow: 0.0,
            open: false,
            hold_left: 0,
            gain_db: p.get(gp::RANGE),
        }
    }

    pub fn adopt(&mut self, old: &mut Gate) {
        self.bank.adopt(&old.bank);
        self.hpf.set_state(old.hpf.state());
        self.lpf.set_state(old.lpf.state());
        self.follow = old.follow;
        self.open = old.open;
        self.hold_left = old.hold_left;
        self.gain_db = old.gain_db;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let follow_rel = time_coef(10.0, self.rate);
        for c in cx.chunks(frames) {
            let (off, n) = (c.off, c.n);
            self.bank.enter(&c);
            let b = &self.bank;
            let expander = b.get(gp::MODE) >= 0.5;
            let t = b.get(gp::THRESHOLD);
            let range = b.get(gp::RANGE);
            let ratio = b.get(gp::RATIO);
            let att = time_coef(b.get(gp::ATTACK), self.rate);
            let rel = time_coef(b.get(gp::RELEASE), self.rate);
            let hold = (b.get(gp::HOLD) * 0.001 * f64::from(self.rate)) as u64;
            let hyst = b.get(gp::HYSTERESIS);
            let (h, l) = (b.get(gp::SC_HPF), b.get(gp::SC_LPF));
            if (h, l) != self.filters {
                let q = std::f64::consts::FRAC_1_SQRT_2;
                self.hpf.c = if h > 20.5 {
                    Coefs::high_pass(h, q, self.rate)
                } else {
                    Coefs::IDENTITY
                };
                self.lpf.c = if l < 19_900.0 {
                    Coefs::low_pass(l, q, self.rate)
                } else {
                    Coefs::IDENTITY
                };
                self.filters = (h, l);
            }
            let key = key(cx, b.get(gp::EXTERNAL) >= 0.5, off, n);
            let piece = &mut buf[off * 2..(off + n) * 2];
            let mut worst = 0.0f64;
            for (i, f) in piece.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let src = key.map_or([f[0], f[1]], |k| [k[i * 2], k[i * 2 + 1]]);
                let mut peak = 0.0f64;
                for ch in 0..2 {
                    let d = self.lpf.run(ch, self.hpf.run(ch, f64::from(src[ch])));
                    peak = peak.max(d.abs());
                }
                self.follow = if peak > self.follow {
                    peak
                } else {
                    peak + (self.follow - peak) * follow_rel
                };
                let x = level_db(self.follow);
                if x > t {
                    self.open = true;
                    self.hold_left = hold;
                } else if x < t - hyst {
                    if self.hold_left > 0 {
                        self.hold_left -= 1;
                    } else {
                        self.open = false;
                    }
                }
                let target = if expander {
                    if self.open || self.hold_left > 0 || x >= t {
                        0.0
                    } else {
                        ((x - t) * (ratio - 1.0)).max(range)
                    }
                } else if self.open {
                    0.0
                } else {
                    range
                };
                let coef = if target > self.gain_db { att } else { rel };
                self.gain_db = target + (self.gain_db - target) * coef;
                worst = worst.max(-self.gain_db);
                let g = db(self.gain_db) as f32;
                f[0] *= g;
                f[1] *= g;
            }
            self.hpf.flush();
            self.lpf.flush();
            self.follow = super::flush64(self.follow);
            self.reduction = self.reduction.max(worst as f32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Effect;
    use super::super::testing::*;
    use super::*;

    fn steady_gain_db(slot: &elyvon_project::effects::EffectSlot, level: f32) -> f64 {
        // A constant (DC) level makes the detector exact in both modes.
        let rate = 48_000;
        let m = map(120.0);
        let x = vec![level; rate as usize];
        let out = run(&mut Effect::new(slot, rate), &x, rate, 128, &m);
        20.0 * f64::from(out[out.len() - 2] / level).log10()
    }

    /// Compression of a test signal follows the static curve.
    #[test]
    fn compressor_matches_its_static_curve() {
        for (threshold, ratio, knee) in [(-20.0, 4.0, 0.0), (-30.0, 2.0, 12.0), (-10.0, 20.0, 6.0)]
        {
            for detector in [0.0, 1.0] {
                let s = slot(
                    EffectKind::Compressor,
                    &[
                        ("thresholdDb", threshold),
                        ("ratio", ratio),
                        ("kneeDb", knee),
                        ("attackMs", 1.0),
                        ("releaseMs", 50.0),
                        ("detector", detector),
                    ],
                );
                for level_dbfs in [-40.0f64, -25.0, -15.0, -6.0, 0.0] {
                    let lin = db(level_dbfs) as f32;
                    let measured = steady_gain_db(&s, lin);
                    let expected =
                        compressor_curve(level_dbfs, threshold, ratio, knee) - level_dbfs;
                    assert!(
                        (measured - expected).abs() < 0.05,
                        "T {threshold} R {ratio} K {knee} det {detector} @ {level_dbfs}: {measured:.3} vs {expected:.3}"
                    );
                }
            }
        }
    }

    #[test]
    fn attack_and_release_follow_their_time_constants() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::Compressor,
            &[
                ("thresholdDb", -20.0),
                ("ratio", 20.0),
                ("kneeDb", 0.0),
                ("attackMs", 10.0),
                ("releaseMs", 100.0),
            ],
        );
        let mut fx = Effect::new(&s, rate);
        // Step from −40 to 0 dBFS: after one attack time constant the
        // reduction reaches 1 − 1/e of its final value.
        let mut x = vec![0.01f32; 4_800 * 2];
        x.extend(std::iter::repeat_n(1.0f32, 48_000 * 2));
        let out = run(&mut fx, &x, rate, 64, &m);
        let final_gr = -(compressor_curve(0.0, -20.0, 20.0, 0.0));
        let at_tau = -20.0 * f64::from(out[(4_800 + 480) * 2]).log10();
        let ratio = at_tau / final_gr;
        assert!((ratio - (1.0 - (-1.0f64).exp())).abs() < 0.03, "{ratio}");
    }

    #[test]
    fn external_key_ducks_the_signal_and_reports_reduction() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::Compressor,
            &[
                ("thresholdDb", -30.0),
                ("ratio", 10.0),
                ("external", 1.0),
                ("attackMs", 1.0),
            ],
        );
        let mut fx = Effect::new(&s, rate);
        let music = sine(300.0, rate, 9_600, 0.1);
        let kick = vec![0.8f32; 9_600 * 2];
        let mut out = music.clone();
        let c = FxContext {
            key: Some(&kick),
            ..cx(&m, rate, 0)
        };
        fx.process(&mut out, &c);
        let gr = fx.take_reduction().expect("dynamics meter");
        assert!(gr > 20.0, "{gr}");
        assert!(rms(&out[4_800..]) < rms(&music[4_800..]) * 0.1);
        assert_eq!(fx.take_reduction(), Some(0.0), "the meter is read once");
        // Without a routed key the detector falls back to the signal itself.
        let mut fx = Effect::new(&s, rate);
        let mut out = music.clone();
        fx.process(&mut out, &cx(&m, rate, 0));
        let internal = slot(
            EffectKind::Compressor,
            &[("thresholdDb", -30.0), ("ratio", 10.0), ("attackMs", 1.0)],
        );
        let mut fx = Effect::new(&internal, rate);
        let mut reference = music.clone();
        fx.process(&mut reference, &cx(&m, rate, 0));
        assert_eq!(out, reference);
    }

    #[test]
    fn parallel_mix_blends_dry_and_compressed() {
        let s = slot(
            EffectKind::Compressor,
            &[
                ("thresholdDb", -40.0),
                ("ratio", 20.0),
                ("kneeDb", 0.0),
                ("mix", 50.0),
            ],
        );
        let g = steady_gain_db(&s, 0.5);
        let wet = db(compressor_curve(-6.0206, -40.0, 20.0, 0.0) + 6.0206);
        let expected = 20.0 * (0.5 * (1.0 + wet)).log10();
        assert!((g - expected).abs() < 0.05, "{g} vs {expected}");
    }

    #[test]
    fn gate_closes_to_its_range_and_opens_above_threshold() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::Gate,
            &[
                ("thresholdDb", -30.0),
                ("rangeDb", -60.0),
                ("holdMs", 0.0),
                ("releaseMs", 5.0),
                ("attackMs", 0.05),
            ],
        );
        let mut fx = Effect::new(&s, rate);
        let mut x = sine(1_000.0, rate, 9_600, 0.001); // −60 dBFS: closed
        x.extend(sine(1_000.0, rate, 9_600, 0.5)); // open
        let out = run(&mut fx, &x, rate, 32, &m);
        let closed = rms(&out[4_800..9_600 * 2]) / rms(&x[4_800..9_600 * 2]);
        let open = rms(&out[(9_600 + 2_400) * 2..]) / rms(&x[(9_600 + 2_400) * 2..]);
        assert!(
            (20.0 * f64::from(closed).log10() + 60.0).abs() < 1.0,
            "{closed}"
        );
        assert!((open - 1.0).abs() < 0.01, "{open}");
        assert!(fx.take_reduction().expect("meter") > 55.0);
    }

    #[test]
    fn expander_reduces_below_threshold_by_its_ratio() {
        let s = slot(
            EffectKind::Gate,
            &[
                ("mode", 1.0),
                ("thresholdDb", -20.0),
                ("ratio", 3.0),
                ("rangeDb", -80.0),
                ("holdMs", 0.0),
            ],
        );
        // 10 dB under the threshold → 20 dB more reduction at 1:3.
        let g = steady_gain_db(&s, db(-30.0) as f32);
        assert!((g + 20.0).abs() < 0.3, "{g}");
        let above = steady_gain_db(&s, db(-10.0) as f32);
        assert!(above.abs() < 0.01, "{above}");
    }
}
