//! Algorithmic reverb: an original eight-line feedback delay network.
//!
//! Structure (textbook FDN, Jot & Chaigne 1991): pre-delay → four series
//! all-pass diffusers → eight delay lines mixed by a Householder matrix
//! (orthogonal, so the loop itself neither gains nor loses energy). Each
//! line has the gain that gives the requested RT60 for its length and a
//! one-pole low-pass for high-frequency damping. Two lines are slowly
//! modulated to avoid metallic ringing. Line lengths are mutually prime and
//! scale with *size*. The tail is stable for every setting: line gains are
//! ≤ 1 and the recursive reads are linear (gain ≤ 1); *freeze* holds the
//! tail with a loop gain of 0.99995.

use elyvon_project::effects::{EffectKind, ParamValues};

use super::delay::{Line, balanced_mix};
use super::filters::OnePole;
use super::{Bank, FxContext, Ramp};

const LINES: usize = 8;
/// Line lengths at 100 % size, ms (mutually prime at 48 kHz).
const LENGTHS_MS: [f64; LINES] = [41.1, 47.3, 53.9, 61.7, 68.3, 77.9, 86.3, 97.1];
const DIFFUSERS_MS: [f64; 4] = [4.77, 3.59, 12.73, 9.31];
const MAX_PREDELAY_MS: f64 = 250.0;
/// Smallest size scale (10 % → 0.25 of the full lengths).
const MIN_SCALE: f64 = 0.25;

mod rp {
    pub const PREDELAY: usize = 0;
    pub const DECAY: usize = 1;
    pub const SIZE: usize = 2;
    pub const DAMPING: usize = 3;
    pub const LOW_CUT: usize = 4;
    pub const DIFFUSION: usize = 5;
    pub const MODULATION: usize = 6;
    pub const WIDTH: usize = 7;
    pub const MIX: usize = 8;
    pub const FREEZE: usize = 9;
}

#[derive(Debug, Clone)]
struct Allpass {
    buf: Vec<f32>,
    at: usize,
}

impl Allpass {
    fn new(frames: usize) -> Self {
        Self {
            buf: vec![0.0; frames.max(1)],
            at: 0,
        }
    }

    #[inline]
    fn run(&mut self, x: f32, g: f32) -> f32 {
        let d = self.buf[self.at];
        let w = x + g * d;
        let y = d - g * w;
        self.buf[self.at] = super::flush(w);
        self.at = (self.at + 1) % self.buf.len();
        y
    }
}

#[derive(Debug, Clone)]
pub struct Reverb {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    predelay: Line,
    diffusers: [Allpass; 4],
    /// One stereo `Line` per pair of FDN lines (channel = line parity).
    lines: Vec<Line>,
    damp: [OnePole; LINES],
    low_cut: [OnePole; 2],
    damping: f64,
    low: f64,
    lfo: [f64; 2],
    mix: [Ramp; 2],
    width: Ramp,
}

impl Reverb {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        let fr = |ms: f64| (ms * 0.001 * f64::from(rate)).ceil() as usize;
        let mut r = Self {
            bank: Bank::new(EffectKind::Reverb, p, rate),
            out_peak: 0.0,
            rate,
            predelay: Line::new(fr(MAX_PREDELAY_MS) + 8),
            diffusers: std::array::from_fn(|i| Allpass::new(fr(DIFFUSERS_MS[i]))),
            lines: (0..LINES / 2)
                .map(|i| Line::new(fr(LENGTHS_MS[2 * i + 1] * 1.02) + 16))
                .collect(),
            damp: [OnePole::default(); LINES],
            low_cut: [OnePole::default(); 2],
            damping: 0.0,
            low: 0.0,
            lfo: [0.0, 0.31],
            mix: [Ramp::hold(0.0); 2],
            width: Ramp::hold((p.get(rp::WIDTH) / 100.0) as f32),
        };
        r.mix = r.mix_gains().map(Ramp::hold);
        r
    }

    fn mix_gains(&self) -> [f32; 2] {
        let (d, w) = balanced_mix((self.bank.get(rp::MIX) / 100.0) as f32);
        [d, w]
    }

    pub fn adopt(&mut self, old: &mut Reverb) {
        self.bank.adopt(&old.bank);
        self.predelay.adopt(&mut old.predelay);
        for (a, b) in self.diffusers.iter_mut().zip(old.diffusers.iter_mut()) {
            if a.buf.len() == b.buf.len() {
                std::mem::swap(&mut a.buf, &mut b.buf);
                a.at = b.at;
            }
        }
        for (a, b) in self.lines.iter_mut().zip(old.lines.iter_mut()) {
            a.adopt(b);
        }
        self.damp = old.damp;
        self.low_cut = old.low_cut;
        self.lfo = old.lfo;
        self.mix = old.mix;
        self.width = old.width;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let rate = f64::from(self.rate);
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let b = &self.bank;
            let freeze = b.get(rp::FREEZE) >= 0.5;
            let scale = MIN_SCALE + (1.0 - MIN_SCALE) * (b.get(rp::SIZE) - 10.0) / 90.0;
            let lengths: [f32; LINES] =
                std::array::from_fn(|i| (LENGTHS_MS[i] * scale * 0.001 * rate) as f32);
            let rt60 = b.get(rp::DECAY);
            let gains: [f32; LINES] = std::array::from_fn(|i| {
                if freeze {
                    0.999_95
                } else {
                    10f64.powf(-3.0 * f64::from(lengths[i]) / (rate * rt60)) as f32
                }
            });
            let damping = if freeze { 20_000.0 } else { b.get(rp::DAMPING) };
            if damping != self.damping {
                for d in &mut self.damp {
                    d.set_freq(damping.min(rate * 0.45), self.rate);
                }
                self.damping = damping;
            }
            let low = b.get(rp::LOW_CUT);
            if low != self.low {
                for d in &mut self.low_cut {
                    d.set_freq(low, self.rate);
                }
                self.low = low;
            }
            let pre = (b.get(rp::PREDELAY) * 0.001 * rate).max(2.0) as f32;
            let diff = (0.75 * b.get(rp::DIFFUSION) / 100.0) as f32;
            let depth = (b.get(rp::MODULATION) / 100.0 * 0.0006 * rate) as f32;
            let input_gain = if freeze { 0.0 } else { 1.0 };
            let width_to = (b.get(rp::WIDTH) / 100.0) as f32;
            let target = self.mix_gains();
            let dry = self.mix[0].step(fresh, target[0]);
            let wet = self.mix[1].step(fresh, target[1]);
            let width = self.width.step(fresh, width_to);
            let tau = std::f64::consts::TAU;
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let mono = 0.5 * (f[0] + f[1]) * input_gain;
                let mut x = self.predelay.read_linear(pre, 0);
                self.predelay.write([mono, 0.0]);
                for d in &mut self.diffusers {
                    x = d.run(x, diff);
                }
                self.lfo[0] = (self.lfo[0] + 0.37 / rate).fract();
                self.lfo[1] = (self.lfo[1] + 0.53 / rate).fract();
                let m0 = depth * (tau * self.lfo[0]).sin() as f32;
                let m1 = depth * (tau * self.lfo[1]).sin() as f32;
                // Read every line, damp and scale it.
                let mut out = [0.0f32; LINES];
                for (k, o) in out.iter_mut().enumerate() {
                    let wobble = match k {
                        2 => m0,
                        5 => m1,
                        _ => 0.0,
                    };
                    let v = self.lines[k / 2].read_linear(lengths[k] + wobble, k % 2);
                    *o = self.damp[k].lp(v) * gains[k];
                }
                // Householder feedback: y = x − (2/N)·Σx, plus the input.
                let sum: f32 = out.iter().sum::<f32>() * (2.0 / LINES as f32);
                let mut feed = [0.0f32; LINES];
                for (k, v) in feed.iter_mut().enumerate() {
                    let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
                    *v = out[k] - sum + x * sign * 0.5;
                }
                for p in 0..LINES / 2 {
                    self.lines[p].write([feed[2 * p], feed[2 * p + 1]]);
                }
                let l = 0.35 * (out[0] - out[2] + out[4] - out[6] + out[1] + out[5]);
                let r = 0.35 * (out[1] - out[3] + out[5] - out[7] + out[2] + out[6]);
                let l = self.low_cut[0].hp(l);
                let r = self.low_cut[1].hp(r);
                let j = ck.idx(i);
                let w = width.at(j, ck.len);
                let m = 0.5 * (l + r);
                let s = 0.5 * (l - r) * w;
                let (dg, wg) = (dry.at(j, ck.len), wet.at(j, ck.len));
                f[0] = f[0] * dg + (m + s) * wg;
                f[1] = f[1] * dg + (m - s) * wg;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Effect;
    use super::super::testing::*;
    use super::*;

    /// Reverberation time from the impulse response's energy decay curve
    /// (Schroeder backward integration, T30 fit from −5 to −35 dB).
    fn t30(ir: &[f32], rate: u32) -> f64 {
        let e: Vec<f64> = ir
            .chunks(2)
            .map(|f| f64::from(f[0]).powi(2) + f64::from(f[1]).powi(2))
            .collect();
        let mut edc = vec![0.0; e.len()];
        let mut acc = 0.0;
        for i in (0..e.len()).rev() {
            acc += e[i];
            edc[i] = acc;
        }
        let total = edc[0];
        let at = |db: f64| {
            edc.iter()
                .position(|v| 10.0 * (v / total).log10() < db)
                .unwrap_or(edc.len())
        };
        let (a, b) = (at(-5.0), at(-35.0));
        (b - a) as f64 / f64::from(rate) * 2.0
    }

    fn ir(values: &[(&str, f64)], rate: u32, seconds: f64) -> Vec<f32> {
        let m = map(120.0);
        let s = slot(EffectKind::Reverb, values);
        let frames = (seconds * f64::from(rate)) as usize;
        let mut x = vec![0.0f32; frames * 2];
        x[0] = 1.0;
        x[1] = 1.0;
        run(&mut Effect::new(&s, rate), &x, rate, 128, &m)
    }

    #[test]
    fn tail_decays_at_the_requested_rt60_at_any_rate() {
        for rate in [44_100u32, 48_000, 96_000] {
            for rt in [0.8, 2.0, 4.0] {
                let out = ir(
                    &[
                        ("decaySeconds", rt),
                        ("mix", 100.0),
                        ("dampingHz", 20_000.0),
                        ("diffusion", 50.0),
                        ("modulation", 0.0),
                        ("predelayMs", 0.0),
                        ("lowCutHz", 20.0),
                    ],
                    rate,
                    rt * 1.6,
                );
                let measured = t30(&out, rate);
                assert!(
                    (measured / rt - 1.0).abs() < 0.2,
                    "{rate} Hz, RT {rt}: measured {measured:.2}"
                );
            }
        }
    }

    #[test]
    fn freeze_holds_and_never_explodes() {
        let rate = 48_000;
        let m = map(120.0);
        let mut fx = Effect::new(
            &slot(
                EffectKind::Reverb,
                &[("mix", 100.0), ("decaySeconds", 20.0)],
            ),
            rate,
        );
        let mut x = sine(300.0, rate, rate as usize, 0.8);
        fx.process(&mut x, &cx(&m, rate, 0));
        fx.set(rp::FREEZE, 1.0);
        let mut held = vec![0.0f32; 10 * rate as usize * 2];
        fx.process(&mut held, &cx(&m, rate, u64::from(rate)));
        let first = rms(&held[..rate as usize * 2]);
        let last = rms(&held[held.len() - rate as usize * 2..]);
        assert!(
            first > 0.01 && last > first * 0.5 && last < first * 1.5,
            "{first} {last}"
        );
        assert!(held.iter().all(|v| v.is_finite() && v.abs() < 8.0));
        assert!(fx.ringing(), "a held tail keeps the strip running");
    }

    #[test]
    fn predelay_delays_the_onset() {
        let rate = 48_000;
        let out = ir(
            &[("predelayMs", 100.0), ("mix", 100.0), ("diffusion", 0.0)],
            rate,
            0.5,
        );
        let onset = out.chunks(2).position(|f| f[0].abs() > 1e-4).unwrap_or(0);
        assert!(onset >= 4_800, "{onset}");
    }
}
