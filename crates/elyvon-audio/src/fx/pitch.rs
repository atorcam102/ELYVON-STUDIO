//! Simple pitch shifter: two read taps sweep through a delay line at the
//! speed set by the pitch ratio and crossfade with complementary sin²/cos²
//! windows half a window apart (a classic time-domain "rotating tape head"
//! design). Formants move with the pitch and transients may double; it is
//! meant for harmonies, octaves and detuning, not transparent correction.
//! The signal is delayed by between 0 and one window (average half a
//! window); this varying delay is not reported as fixed latency.

use elyvon_project::effects::{EffectKind, ParamValues};

use super::delay::Line;
use super::{Bank, FxContext, Ramp};

mod pp {
    pub const SEMITONES: usize = 0;
    pub const CENTS: usize = 1;
    pub const WINDOW: usize = 2;
    pub const MIX: usize = 3;
}

const MAX_WINDOW_MS: f64 = 100.0;

#[derive(Debug, Clone)]
pub struct PitchShifter {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    line: Line,
    /// Position of the first tap in the window, 0…1.
    phase: f64,
    mix: Ramp,
}

impl PitchShifter {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        Self {
            bank: Bank::new(EffectKind::PitchShifter, p, rate),
            out_peak: 0.0,
            rate,
            line: Line::new((MAX_WINDOW_MS * 0.001 * f64::from(rate)) as usize + 16),
            phase: 0.0,
            mix: Ramp::hold((p.get(pp::MIX) / 100.0) as f32),
        }
    }

    pub fn adopt(&mut self, old: &mut PitchShifter) {
        self.bank.adopt(&old.bank);
        self.line.adopt(&mut old.line);
        self.phase = old.phase;
        self.mix = old.mix;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let b = &self.bank;
            let ratio = 2f64.powf((b.get(pp::SEMITONES) + b.get(pp::CENTS) / 100.0) / 12.0);
            let window = (b.get(pp::WINDOW) * 0.001 * f64::from(self.rate)).max(8.0);
            // The tap delay shrinks by (ratio − 1) frames per frame.
            let step = (1.0 - ratio) / window;
            let mix_to = (b.get(pp::MIX) / 100.0) as f32;
            let mix = self.mix.step(fresh, mix_to);
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                self.phase = (self.phase + step).rem_euclid(1.0);
                let p2 = (self.phase + 0.5).rem_euclid(1.0);
                let (d1, d2) = (
                    (2.0 + self.phase * window) as f32,
                    (2.0 + p2 * window) as f32,
                );
                let g1 = (std::f64::consts::PI * self.phase).sin().powi(2) as f32;
                let g2 = 1.0 - g1;
                let m = mix.at(ck.idx(i), ck.len);
                let mut out = [0.0f32; 2];
                for (ch, o) in out.iter_mut().enumerate() {
                    *o = self.line.read(d1, ch) * g1 + self.line.read(d2, ch) * g2;
                }
                self.line.write([f[0], f[1]]);
                f[0] += (out[0] - f[0]) * m;
                f[1] += (out[1] - f[1]) * m;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Effect;
    use super::super::testing::*;
    use super::*;

    /// Strongest frequency between 100 Hz and 1 kHz (Goertzel scan, 1 Hz steps).
    fn frequency(x: &[f32], rate: u32) -> f64 {
        let l: Vec<f64> = x.chunks(2).map(|f| f64::from(f[0])).collect();
        let power = |freq: f64| {
            let c = 2.0 * (2.0 * std::f64::consts::PI * freq / f64::from(rate)).cos();
            let (mut s1, mut s2) = (0.0f64, 0.0f64);
            for v in &l {
                let s = v + c * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            s1 * s1 + s2 * s2 - c * s1 * s2
        };
        (100..1_000)
            .map(f64::from)
            .max_by(|a, b| power(*a).total_cmp(&power(*b)))
            .unwrap_or(0.0)
    }

    #[test]
    fn shifts_by_the_requested_interval() {
        let rate = 48_000;
        let m = map(120.0);
        let x = sine(220.0, rate, rate as usize, 0.5);
        for (semis, cents) in [(12.0, 0.0), (-12.0, 0.0), (7.0, 0.0), (0.0, 50.0)] {
            let s = slot(
                EffectKind::PitchShifter,
                &[("semitones", semis), ("cents", cents), ("windowMs", 50.0)],
            );
            let out = run(&mut Effect::new(&s, rate), &x, rate, 128, &m);
            let f = frequency(&out[9_600..], rate);
            let expected = 220.0 * 2f64.powf((semis + cents / 100.0) / 12.0);
            // The crossfade modulates the output at |ratio − 1| / window: the
            // strongest line lies within that sideband of the target.
            let ratio = expected / 220.0;
            let sideband = (ratio - 1.0).abs() * 1000.0 / 50.0;
            assert!(
                (f - expected).abs() <= sideband + 2.0,
                "{semis} st {cents} c: {f:.1} Hz vs {expected:.1}"
            );
        }
    }
}
