//! Eight-band parametric equaliser: high/low-pass (12–48 dB/oct), shelves,
//! bells, band-pass and notch, in double-precision biquad cascades.
//!
//! Band parameters glide (see [`super::Bank`]) and the coefficients are
//! recomputed once per control chunk while they move; switching a band on
//! or off crossfades it over 5 ms. [`response_db`] evaluates the exact
//! transfer function of the same sections for the editor's curve.

use elyvon_project::effects::{EQ_BAND_PARAMS, EQ_BANDS, EffectKind, ParamValues, eq_type};

use super::filters::{Biquad, Coefs, butterworth_q};
use super::{Bank, CONTROL, FxContext, Ramp, db};

pub const MAX_SECTIONS: usize = 4;
const OUTPUT: usize = EQ_BANDS * EQ_BAND_PARAMS;
const PROCESSING: usize = OUTPUT + 1;

#[derive(Debug, Clone, Copy, PartialEq)]
struct BandSpec {
    ty: usize,
    freq: f64,
    gain: f64,
    q: f64,
    slope: usize,
}

fn spec(values: impl Fn(usize) -> f64, band: usize) -> BandSpec {
    let at = |p: usize| values(band * EQ_BAND_PARAMS + p);
    BandSpec {
        ty: at(1).round().max(0.0) as usize,
        freq: at(2),
        gain: at(3),
        q: at(4),
        slope: at(5).round().clamp(0.0, 3.0) as usize,
    }
}

/// The sections of one band. High/low-pass bands cascade Butterworth
/// sections (one per 12 dB/oct); the band's Q scales the resonance of the
/// last section (Q 0.707 is a plain Butterworth response).
fn sections(s: &BandSpec, rate: u32) -> ([Coefs; MAX_SECTIONS], usize) {
    let mut out = [Coefs::IDENTITY; MAX_SECTIONS];
    let n = match s.ty {
        eq_type::HIGH_PASS | eq_type::LOW_PASS => {
            let n = s.slope + 1;
            for (k, c) in out.iter_mut().enumerate().take(n) {
                let mut q = butterworth_q(n, k);
                if k == n - 1 {
                    q *= s.q / std::f64::consts::FRAC_1_SQRT_2;
                }
                *c = if s.ty == eq_type::HIGH_PASS {
                    Coefs::high_pass(s.freq, q, rate)
                } else {
                    Coefs::low_pass(s.freq, q, rate)
                };
            }
            n
        }
        eq_type::LOW_SHELF => {
            out[0] = Coefs::low_shelf(s.freq, s.q, s.gain, rate);
            1
        }
        eq_type::HIGH_SHELF => {
            out[0] = Coefs::high_shelf(s.freq, s.q, s.gain, rate);
            1
        }
        eq_type::BAND_PASS => {
            out[0] = Coefs::band_pass(s.freq, s.q, rate);
            1
        }
        eq_type::NOTCH => {
            out[0] = Coefs::notch(s.freq, s.q, rate);
            1
        }
        _ => {
            out[0] = Coefs::peak(s.freq, s.q, s.gain, rate);
            1
        }
    };
    (out, n)
}

#[derive(Debug, Clone)]
struct Band {
    spec: Option<BandSpec>,
    sections: [Biquad; MAX_SECTIONS],
    count: usize,
    identity: bool,
    /// Crossfade 0 (bypassed) … 1 (active).
    mix: f32,
}

impl Band {
    fn update(&mut self, s: BandSpec, rate: u32) {
        if self.spec == Some(s) {
            return;
        }
        let (c, n) = sections(&s, rate);
        if n != self.count {
            for b in &mut self.sections {
                b.reset();
            }
        }
        for (b, c) in self.sections.iter_mut().zip(c) {
            b.c = c;
        }
        self.count = n;
        self.identity = n == 1 && c[0] == Coefs::IDENTITY;
        self.spec = Some(s);
    }
}

#[derive(Debug, Clone)]
pub struct ParametricEq {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    bands: [Band; EQ_BANDS],
    gain: f32,
    /// Per-chunk step of a band crossfade (5 ms).
    fade_step: f32,
    /// Values of the current control chunk.
    fades: [Ramp; EQ_BANDS],
    gain_ramp: Ramp,
}

impl ParametricEq {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        let bank = Bank::new(EffectKind::ParametricEq, p, rate);
        let band = Band {
            spec: None,
            sections: [Biquad::default(); MAX_SECTIONS],
            count: 1,
            identity: true,
            mix: 0.0,
        };
        let mut eq = Self {
            bank,
            out_peak: 0.0,
            rate,
            bands: std::array::from_fn(|_| band.clone()),
            gain: db(p.get(OUTPUT)) as f32,
            fade_step: (CONTROL as f64 / (0.005 * f64::from(rate))).min(1.0) as f32,
            fades: [Ramp { from: 0.0, to: 0.0 }; EQ_BANDS],
            gain_ramp: Ramp { from: 1.0, to: 1.0 },
        };
        for b in 0..EQ_BANDS {
            let s = spec(|i| eq.bank.get(i), b);
            eq.bands[b].update(s, rate);
            eq.bands[b].mix = if eq.bank.get(b * EQ_BAND_PARAMS) >= 0.5 {
                1.0
            } else {
                0.0
            };
        }
        eq
    }

    pub fn adopt(&mut self, old: &mut ParametricEq) {
        self.bank.adopt(&old.bank);
        self.gain = old.gain;
        self.fades = old.fades;
        self.gain_ramp = old.gain_ramp;
        for (a, b) in self.bands.iter_mut().zip(&old.bands) {
            a.mix = b.mix;
            if a.count == b.count {
                for (x, y) in a.sections.iter_mut().zip(&b.sections) {
                    x.set_state(y.state());
                }
            }
            // Recompute from the heard values on the next chunk.
            a.spec = None;
        }
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        for c in cx.chunks(frames) {
            let (off, n) = (c.off, c.n);
            if self.bank.enter(&c) {
                self.chunk();
            }
            let fades = self.fades;
            let g = self.gain_ramp;
            // 0: both channels; 1: the mid signal; 2: the side signal.
            let mode = self.bank.get(PROCESSING).round() as usize;
            let channels: &[usize] = match mode {
                0 => &[0, 1],
                1 => &[0],
                _ => &[1],
            };
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let j = c.idx(i);
                let (l, r) = (f64::from(f[0]), f64::from(f[1]));
                let mut v = if mode == 0 {
                    [l, r]
                } else {
                    [0.5 * (l + r), 0.5 * (l - r)]
                };
                for &ch in channels {
                    let mut x = v[ch];
                    for (band, fade) in self.bands.iter_mut().zip(&fades) {
                        if band.identity || (fade.from == 0.0 && fade.to == 0.0) {
                            continue;
                        }
                        let mut y = x;
                        for sec in band.sections.iter_mut().take(band.count) {
                            y = sec.run(ch, y);
                        }
                        let m = f64::from(fade.at(j, c.len));
                        x += (y - x) * m;
                    }
                    v[ch] = x;
                }
                let (ol, or) = if mode == 0 {
                    (v[0], v[1])
                } else {
                    (v[0] + v[1], v[0] - v[1])
                };
                let k = f64::from(g.at(j, c.len));
                f[0] = (ol * k) as f32;
                f[1] = (or * k) as f32;
            }
            for band in &mut self.bands {
                for s in band.sections.iter_mut().take(band.count) {
                    s.flush();
                }
            }
        }
    }

    /// Per-chunk update: coefficients, band crossfades and output gain.
    fn chunk(&mut self) {
        {
            let fades = &mut self.fades;
            for (b, band) in self.bands.iter_mut().enumerate() {
                let s = spec(|i| self.bank.get(i), b);
                band.update(s, self.rate);
                let on = self.bank.get(b * EQ_BAND_PARAMS) >= 0.5;
                let from = band.mix;
                band.mix = if on {
                    (from + self.fade_step).min(1.0)
                } else {
                    (from - self.fade_step).max(0.0)
                };
                fades[b] = Ramp { from, to: band.mix };
                if from == 0.0 && band.mix == 0.0 {
                    for s in &mut band.sections {
                        s.reset();
                    }
                }
            }
        }
        self.gain_ramp = Ramp {
            from: self.gain,
            to: db(self.bank.get(OUTPUT)) as f32,
        };
        self.gain = self.gain_ramp.to;
    }
}

/// Response in dB of each active band (without the output gain), for the
/// editor's per-band areas: `(band index, dB per frequency)`.
pub fn band_responses_db(p: &ParamValues, rate: u32, freqs: &[f64]) -> Vec<(usize, Vec<f64>)> {
    (0..EQ_BANDS)
        .filter(|b| p.get(b * EQ_BAND_PARAMS) >= 0.5)
        .map(|b| {
            let (c, n) = sections(&spec(|i| p.get(i), b), rate);
            let db = freqs
                .iter()
                .map(|&f| {
                    let m: f64 = c.iter().take(n).map(|s| s.magnitude(f, rate)).product();
                    20.0 * m.max(1e-12).log10()
                })
                .collect();
            (b, db)
        })
        .collect()
}

/// Magnitude response in dB of the equaliser described by `p` at each of
/// `freqs`: the product of the active bands' sections and the output gain.
pub fn response_db(p: &ParamValues, rate: u32, freqs: &[f64]) -> Vec<f64> {
    let mut bands = Vec::new();
    for b in 0..EQ_BANDS {
        if p.get(b * EQ_BAND_PARAMS) >= 0.5 {
            let s = spec(|i| p.get(i), b);
            bands.push(sections(&s, rate));
        }
    }
    let out = p.get(OUTPUT);
    freqs
        .iter()
        .map(|&f| {
            let mut m = 1.0;
            for (c, n) in &bands {
                for sec in c.iter().take(*n) {
                    m *= sec.magnitude(f, rate);
                }
            }
            20.0 * m.max(1e-12).log10() + out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::super::{Effect, filters::Coefs};
    use super::*;

    fn gain_at(slot: &elyvon_project::effects::EffectSlot, freq: f64, rate: u32) -> f64 {
        let m = map(120.0);
        let x = sine(freq, rate, rate as usize, 0.25);
        let out = run(&mut Effect::new(slot, rate), &x, rate, 128, &m);
        let from = x.len() / 2;
        20.0 * f64::from(rms(&out[from..]) / rms(&x[from..])).log10()
    }

    /// Frequency sweep: the measured gain of a sine at each frequency matches
    /// the response the editor draws.
    #[test]
    fn measured_sweep_matches_the_drawn_response() {
        let rate = 48_000;
        let s = slot(
            EffectKind::ParametricEq,
            &[
                ("b1On", 1.0),
                ("b1Freq", 60.0),
                ("b1Slope", 1.0),
                ("b2Gain", 4.0),
                ("b2Freq", 150.0),
                ("b4Gain", -8.0),
                ("b4Freq", 900.0),
                ("b4Q", 2.0),
                ("b6Gain", 6.0),
                ("b6Freq", 4_000.0),
                ("b7Gain", -3.0),
                ("b7Freq", 12_000.0),
                ("outputDb", -1.5),
            ],
        );
        let freqs = [
            40.0, 100.0, 300.0, 900.0, 2_000.0, 4_000.0, 8_000.0, 15_000.0,
        ];
        let drawn = response_db(&s.params, rate, &freqs);
        for (f, d) in freqs.iter().zip(&drawn) {
            let m = gain_at(&s, *f, rate);
            assert!(
                (m - d).abs() < 0.1,
                "{f} Hz: measured {m:.2} dB, drawn {d:.2} dB"
            );
        }
    }

    #[test]
    fn slopes_give_12_to_48_db_per_octave() {
        let rate = 48_000;
        for (slope, per_octave) in [(0.0, 12.0), (1.0, 24.0), (2.0, 36.0), (3.0, 48.0)] {
            // Far below Nyquist the bilinear transform keeps the analogue slope.
            let s = slot(
                EffectKind::ParametricEq,
                &[("b8On", 1.0), ("b8Freq", 200.0), ("b8Slope", slope)],
            );
            let r = response_db(&s.params, rate, &[1_600.0, 3_200.0]);
            let measured = r[0] - r[1];
            assert!(
                (measured - per_octave).abs() < 1.0,
                "slope {slope}: {measured}"
            );
            let at_cutoff = response_db(&s.params, rate, &[200.0])[0];
            assert!(
                (at_cutoff + 3.01).abs() < 0.1,
                "Butterworth −3 dB: {at_cutoff}"
            );
        }
    }

    #[test]
    fn mid_and_side_processing_touch_only_their_component() {
        let rate = 48_000;
        let m = map(120.0);
        let boost = |processing: f64| {
            slot(
                EffectKind::ParametricEq,
                &[
                    ("b4Gain", 12.0),
                    ("b4Freq", 1_000.0),
                    ("processing", processing),
                ],
            )
        };
        // A mono (pure mid) and an anti-phase (pure side) 1 kHz tone.
        let mono = sine(1_000.0, rate, 24_000, 0.1);
        let side: Vec<f32> = mono.chunks(2).flat_map(|f| [f[0], -f[1]]).collect();
        let gain = |s: &elyvon_project::effects::EffectSlot, x: &[f32]| {
            let out = run(&mut Effect::new(s, rate), x, rate, 128, &m);
            20.0 * f64::from(rms(&out[24_000..]) / rms(&x[24_000..])).log10()
        };
        assert!((gain(&boost(0.0), &mono) - 12.0).abs() < 0.1);
        assert!(
            (gain(&boost(1.0), &mono) - 12.0).abs() < 0.1,
            "mid boosts mono"
        );
        assert!(
            gain(&boost(1.0), &side).abs() < 0.01,
            "mid leaves the side alone"
        );
        assert!(
            (gain(&boost(2.0), &side) - 12.0).abs() < 0.1,
            "side boosts anti-phase"
        );
        assert!(
            gain(&boost(2.0), &mono).abs() < 0.01,
            "side leaves the mid alone"
        );
        // Per-band curves add up to the drawn total (minus the output gain).
        let s = slot(
            EffectKind::ParametricEq,
            &[("b3Gain", 5.0), ("b6Gain", -4.0), ("outputDb", 2.0)],
        );
        let f = [100.0, 250.0, 1_000.0, 6_000.0];
        let total = response_db(&s.params, rate, &f);
        let bands = band_responses_db(&s.params, rate, &f);
        for (k, t) in total.iter().enumerate() {
            let sum: f64 = bands.iter().map(|(_, db)| db[k]).sum::<f64>() + 2.0;
            assert!((sum - t).abs() < 1e-9);
        }
    }

    #[test]
    fn band_switching_is_click_free() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::ParametricEq,
            &[("b4Gain", 18.0), ("b4Freq", 1_000.0)],
        );
        let mut fx = Effect::new(&s, rate);
        let x = sine(1_000.0, rate, 4_800, 0.1);
        let mut out = x.clone();
        let (a, b) = out.split_at_mut(2_400 * 2);
        fx.process(a, &cx(&m, rate, 0));
        fx.set(3 * 6, 0.0); // band 4 off
        fx.process(b, &cx(&m, rate, 2_400));
        // The largest frame-to-frame step stays that of a smooth sine.
        let step = out
            .chunks(2)
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs())
            .fold(0f32, f32::max);
        let sine_step = 0.1 * 7.94 * (2.0 * std::f32::consts::PI * 1_000.0 / 48_000.0);
        assert!(step <= sine_step * 1.05, "{step} vs {sine_step}");
    }

    #[test]
    fn extreme_low_band_is_stable_and_accurate() {
        let rate = 192_000;
        let m = map(120.0);
        // The narrowest, loudest 20 Hz bell: stable poles, finite output.
        let c = Coefs::peak(20.0, 18.0, 24.0, rate);
        assert!(c.is_stable());
        let s = slot(
            EffectKind::ParametricEq,
            &[("b3Freq", 20.0), ("b3Gain", 24.0), ("b3Q", 18.0)],
        );
        let x = sine(20.0, rate, rate as usize, 0.5);
        let out = run(&mut Effect::new(&s, rate), &x, rate, 128, &m);
        assert!(out.iter().all(|v| v.is_finite()));
        // A Q 4 bell settles within 3 s and reaches its gain exactly.
        let s = slot(
            EffectKind::ParametricEq,
            &[("b3Freq", 20.0), ("b3Gain", 24.0), ("b3Q", 4.0)],
        );
        let x = sine(20.0, rate, 3 * rate as usize, 0.01);
        let out = run(&mut Effect::new(&s, rate), &x, rate, 128, &m);
        let from = x.len() * 5 / 6;
        let g = 20.0 * f64::from(rms(&out[from..]) / rms(&x[from..])).log10();
        assert!((g - 24.0).abs() < 0.3, "{g}");
    }
}
