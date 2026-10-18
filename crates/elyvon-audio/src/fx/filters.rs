//! Second-order IIR sections (RBJ audio-EQ cookbook designs) in double
//! precision, one-pole filters and their exact frequency responses.
//!
//! Sections run in transposed direct form II with `f64` state: at 48 kHz a
//! 20 Hz band has poles within 0.3 % of the unit circle, where `f32`
//! coefficients and state lose several dB of accuracy and add noise.

use std::f64::consts::PI;

/// Normalised biquad coefficients (`a0 = 1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coefs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coefs {
    pub const IDENTITY: Coefs = Coefs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn norm(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    fn omega(freq: f64, rate: u32) -> (f64, f64) {
        let nyquist = f64::from(rate) * 0.5;
        let f = freq.clamp(1.0, nyquist * 0.98);
        let w = 2.0 * PI * f / f64::from(rate);
        w.sin_cos()
    }

    pub fn low_pass(freq: f64, q: f64, rate: u32) -> Self {
        let (s, c) = Self::omega(freq, rate);
        let alpha = s / (2.0 * q.max(0.05));
        Self::norm(
            (1.0 - c) / 2.0,
            1.0 - c,
            (1.0 - c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }

    pub fn high_pass(freq: f64, q: f64, rate: u32) -> Self {
        let (s, c) = Self::omega(freq, rate);
        let alpha = s / (2.0 * q.max(0.05));
        Self::norm(
            (1.0 + c) / 2.0,
            -(1.0 + c),
            (1.0 + c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }

    /// Constant 0 dB peak gain band-pass.
    pub fn band_pass(freq: f64, q: f64, rate: u32) -> Self {
        let (s, c) = Self::omega(freq, rate);
        let alpha = s / (2.0 * q.max(0.05));
        Self::norm(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn notch(freq: f64, q: f64, rate: u32) -> Self {
        let (s, c) = Self::omega(freq, rate);
        let alpha = s / (2.0 * q.max(0.05));
        Self::norm(1.0, -2.0 * c, 1.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn peak(freq: f64, q: f64, gain_db: f64, rate: u32) -> Self {
        if gain_db == 0.0 {
            return Self::IDENTITY;
        }
        let a = 10f64.powf(gain_db / 40.0);
        let (s, c) = Self::omega(freq, rate);
        let alpha = s / (2.0 * q.max(0.05));
        Self::norm(
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        )
    }

    pub fn low_shelf(freq: f64, q: f64, gain_db: f64, rate: u32) -> Self {
        if gain_db == 0.0 {
            return Self::IDENTITY;
        }
        let a = 10f64.powf(gain_db / 40.0);
        let (s, c) = Self::omega(freq, rate);
        let alpha = s / (2.0 * q.max(0.05));
        let k = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        )
    }

    pub fn high_shelf(freq: f64, q: f64, gain_db: f64, rate: u32) -> Self {
        if gain_db == 0.0 {
            return Self::IDENTITY;
        }
        let a = 10f64.powf(gain_db / 40.0);
        let (s, c) = Self::omega(freq, rate);
        let alpha = s / (2.0 * q.max(0.05));
        let k = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) + (a - 1.0) * c + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - k),
            (a + 1.0) - (a - 1.0) * c + k,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - k,
        )
    }

    /// First-order sections expressed as biquads (b2 = a2 = 0).
    pub fn low_pass1(freq: f64, rate: u32) -> Self {
        let k = (PI * freq.clamp(1.0, f64::from(rate) * 0.49) / f64::from(rate)).tan();
        Self::norm(k, k, 0.0, k + 1.0, k - 1.0, 0.0)
    }

    pub fn high_pass1(freq: f64, rate: u32) -> Self {
        let k = (PI * freq.clamp(1.0, f64::from(rate) * 0.49) / f64::from(rate)).tan();
        Self::norm(1.0, -1.0, 0.0, k + 1.0, k - 1.0, 0.0)
    }

    /// First-order all-pass with its break at `freq`.
    pub fn all_pass1(freq: f64, rate: u32) -> Self {
        let k = (PI * freq.clamp(1.0, f64::from(rate) * 0.49) / f64::from(rate)).tan();
        let a = (k - 1.0) / (k + 1.0);
        Self::norm(a, 1.0, 0.0, 1.0, a, 0.0)
    }

    /// `|H(e^{jω})|` at `freq`.
    pub fn magnitude(&self, freq: f64, rate: u32) -> f64 {
        let w = 2.0 * PI * freq / f64::from(rate);
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        ((nr * nr + ni * ni) / (dr * dr + di * di).max(1e-300)).sqrt()
    }

    /// Poles inside the unit circle (stability check).
    pub fn is_stable(&self) -> bool {
        self.a2.abs() < 1.0 && self.a1.abs() < 1.0 + self.a2
    }
}

/// A stereo biquad with double-precision state.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    pub c: Coefs,
    z: [[f64; 2]; 2],
}

impl Default for Biquad {
    fn default() -> Self {
        Self::new(Coefs::IDENTITY)
    }
}

impl Biquad {
    pub fn new(c: Coefs) -> Self {
        Self {
            c,
            z: [[0.0; 2]; 2],
        }
    }

    #[inline]
    pub fn run(&mut self, ch: usize, x: f64) -> f64 {
        let c = &self.c;
        let z = &mut self.z[ch];
        let y = c.b0 * x + z[0];
        z[0] = c.b1 * x - c.a1 * y + z[1];
        z[1] = c.b2 * x - c.a2 * y;
        y
    }

    /// Flushes denormal state (call once per control chunk).
    #[inline]
    pub fn flush(&mut self) {
        for z in &mut self.z {
            for v in z.iter_mut() {
                *v = super::flush64(*v);
                if !v.is_finite() {
                    *v = 0.0;
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.z = [[0.0; 2]; 2];
    }

    pub fn state(&self) -> [[f64; 2]; 2] {
        self.z
    }

    pub fn set_state(&mut self, z: [[f64; 2]; 2]) {
        self.z = z;
    }
}

/// One-pole low-pass smoother/filter on one channel (`f32`).
#[derive(Debug, Clone, Copy, Default)]
pub struct OnePole {
    pub a: f32,
    pub z: f32,
}

impl OnePole {
    /// Low-pass with its −3 dB point at `freq`.
    pub fn low_pass(freq: f64, rate: u32) -> Self {
        let a = (-2.0 * PI * freq.clamp(1.0, f64::from(rate) * 0.49) / f64::from(rate)).exp();
        Self {
            a: a as f32,
            z: 0.0,
        }
    }

    pub fn set_freq(&mut self, freq: f64, rate: u32) {
        self.a =
            (-2.0 * PI * freq.clamp(1.0, f64::from(rate) * 0.49) / f64::from(rate)).exp() as f32;
    }

    #[inline]
    pub fn lp(&mut self, x: f32) -> f32 {
        self.z = x + (self.z - x) * self.a;
        self.z = super::flush(self.z);
        self.z
    }

    #[inline]
    pub fn hp(&mut self, x: f32) -> f32 {
        x - self.lp(x)
    }
}

/// Butterworth Q of section `k` of an order-`2n` cascade.
pub fn butterworth_q(n: usize, k: usize) -> f64 {
    let order = (2 * n) as f64;
    1.0 / (2.0 * (PI * (2 * k + 1) as f64 / (2.0 * order)).cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured_gain(c: Coefs, freq: f64, rate: u32) -> f64 {
        let mut b = Biquad::new(c);
        let n = rate as usize;
        let (mut sum_in, mut sum_out) = (0.0, 0.0);
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f64 / f64::from(rate)).sin();
            let y = b.run(0, x);
            if i > n / 2 {
                sum_in += x * x;
                sum_out += y * y;
            }
        }
        (sum_out / sum_in).sqrt()
    }

    #[test]
    fn designs_match_their_analytic_response() {
        let rate = 48_000;
        let cases = [
            (Coefs::peak(1_000.0, 1.0, 12.0, rate), 1_000.0, 12.0),
            (Coefs::peak(1_000.0, 1.0, -12.0, rate), 1_000.0, -12.0),
            (Coefs::low_shelf(200.0, 0.707, 6.0, rate), 20.0, 6.0),
            (
                Coefs::high_shelf(5_000.0, 0.707, -6.0, rate),
                18_000.0,
                -6.0,
            ),
            (Coefs::low_pass(1_000.0, 0.707, rate), 1_000.0, -3.01),
            (Coefs::high_pass(1_000.0, 0.707, rate), 1_000.0, -3.01),
            (Coefs::band_pass(1_000.0, 2.0, rate), 1_000.0, 0.0),
        ];
        for (c, f, expect_db) in cases {
            assert!(c.is_stable());
            let analytic = 20.0 * c.magnitude(f, rate).log10();
            assert!(
                (analytic - expect_db).abs() < 0.15,
                "{c:?} analytic {analytic}"
            );
            let measured = 20.0 * measured_gain(c, f, rate).log10();
            assert!(
                (measured - analytic).abs() < 0.05,
                "{c:?} measured {measured} vs {analytic}"
            );
        }
        let notch = Coefs::notch(1_000.0, 2.0, rate);
        assert!(notch.magnitude(1_000.0, rate) < 1e-6);
    }

    #[test]
    fn low_frequency_sections_stay_accurate_and_stable_at_high_rates() {
        for rate in [44_100, 96_000, 192_000] {
            let c = Coefs::high_pass(20.0, 0.707, rate);
            assert!(c.is_stable());
            let g = 20.0 * measured_gain(c, 20.0, rate).log10();
            assert!((g + 3.01).abs() < 0.1, "{rate}: {g}");
            let p = Coefs::peak(25.0, 4.0, 18.0, rate);
            assert!(p.is_stable());
        }
    }

    #[test]
    fn butterworth_sections() {
        assert!((butterworth_q(1, 0) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert!((butterworth_q(2, 0) - 0.541_196).abs() < 1e-5);
        assert!((butterworth_q(2, 1) - 1.306_563).abs() < 1e-5);
    }
}
