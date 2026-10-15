//! Gain and pan laws shared by the mixer UI (for display) and the future engine.

/// Fader values at or below this level are treated as silence (−∞ dB).
pub const SILENCE_DB: f64 = -96.0;

/// Converts decibels to a linear amplitude factor.
#[inline]
pub fn db_to_gain(db: f64) -> f64 {
    if db <= SILENCE_DB {
        0.0
    } else {
        10f64.powf(db / 20.0)
    }
}

/// Converts a linear amplitude factor to decibels, clamped to [`SILENCE_DB`].
#[inline]
pub fn gain_to_db(gain: f64) -> f64 {
    if gain <= 0.0 {
        SILENCE_DB
    } else {
        (20.0 * gain.log10()).max(SILENCE_DB)
    }
}

/// Constant-power (−3 dB centre) pan law. `pan` is −1 (left) … +1 (right).
/// Returns `(left, right)` gains whose squares sum to 1.
#[inline]
pub fn constant_power_pan(pan: f64) -> (f64, f64) {
    let p = pan.clamp(-1.0, 1.0);
    let angle = (p + 1.0) * std::f64::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_conversions() {
        assert_eq!(db_to_gain(0.0), 1.0);
        assert!((db_to_gain(-6.0) - 0.501_187).abs() < 1e-6);
        assert_eq!(db_to_gain(SILENCE_DB), 0.0);
        assert_eq!(gain_to_db(0.0), SILENCE_DB);
        assert!((gain_to_db(db_to_gain(-12.5)) + 12.5).abs() < 1e-9);
    }

    #[test]
    fn pan_is_constant_power() {
        for pan in [-1.0, -0.5, 0.0, 0.3, 1.0] {
            let (l, r) = constant_power_pan(pan);
            assert!((l * l + r * r - 1.0).abs() < 1e-12);
        }
        let (l, r) = constant_power_pan(-1.0);
        assert!((l - 1.0).abs() < 1e-12 && r.abs() < 1e-12);
        let (l, r) = constant_power_pan(0.0);
        assert!((l - r).abs() < 1e-12);
    }
}
