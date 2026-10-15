//! Musical time, metre and key.
//!
//! Positions are integer ticks at a fixed resolution of [`PPQ`] ticks per
//! quarter note. Integer ticks make editing exact (no floating-point drift when
//! regions are moved back and forth) and convert losslessly between metres.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Ticks per quarter note. 960 divides evenly by 2, 3, 4, 5, 6, 8, 10, 12, 15,
/// 16 and 64th-note triplets, which covers every common grid.
pub const PPQ: u32 = 960;

/// A position or duration in ticks.
pub type Ticks = u64;

/// Tempo in beats (quarter notes) per minute.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
#[serde(transparent)]
#[ts(export, type = "number")]
pub struct Tempo(f64);

impl Tempo {
    pub const MIN_BPM: f64 = 20.0;
    pub const MAX_BPM: f64 = 999.0;
    pub const DEFAULT: Tempo = Tempo(120.0);

    pub fn new(bpm: f64) -> Option<Self> {
        (bpm.is_finite() && (Self::MIN_BPM..=Self::MAX_BPM).contains(&bpm)).then_some(Self(bpm))
    }

    pub fn bpm(self) -> f64 {
        self.0
    }

    pub fn is_valid(self) -> bool {
        Self::new(self.0).is_some()
    }

    /// Duration of `ticks` in seconds at this (constant) tempo.
    pub fn ticks_to_seconds(self, ticks: Ticks) -> f64 {
        ticks as f64 / f64::from(PPQ) * 60.0 / self.0
    }
}

/// A time signature such as 4/4, 6/8 or 7/16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self {
            numerator: 4,
            denominator: 4,
        }
    }
}

impl TimeSignature {
    pub fn new(numerator: u8, denominator: u8) -> Option<Self> {
        let ts = Self {
            numerator,
            denominator,
        };
        ts.is_valid().then_some(ts)
    }

    pub fn is_valid(self) -> bool {
        (1..=32).contains(&self.numerator) && matches!(self.denominator, 1 | 2 | 4 | 8 | 16 | 32)
    }

    /// Ticks in one beat (one `1/denominator` note).
    pub fn ticks_per_beat(self) -> Ticks {
        Ticks::from(PPQ) * 4 / Ticks::from(self.denominator.max(1))
    }

    pub fn ticks_per_bar(self) -> Ticks {
        self.ticks_per_beat() * Ticks::from(self.numerator.max(1))
    }
}

impl fmt::Display for TimeSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.numerator, self.denominator)
    }
}

/// A 1-based bar.beat.tick position, as displayed to musicians.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MusicalPosition {
    #[ts(type = "number")]
    pub bar: u64,
    #[ts(type = "number")]
    pub beat: u64,
    #[ts(type = "number")]
    pub tick: u64,
}

impl MusicalPosition {
    pub fn from_ticks(ticks: Ticks, ts: TimeSignature) -> Self {
        let per_bar = ts.ticks_per_bar();
        let per_beat = ts.ticks_per_beat();
        let in_bar = ticks % per_bar;
        Self {
            bar: ticks / per_bar + 1,
            beat: in_bar / per_beat + 1,
            tick: in_bar % per_beat,
        }
    }

    /// Converts back to ticks. Returns `None` for out-of-range components.
    pub fn to_ticks(self, ts: TimeSignature) -> Option<Ticks> {
        if self.bar == 0
            || self.beat == 0
            || self.beat > u64::from(ts.numerator)
            || self.tick >= ts.ticks_per_beat()
        {
            return None;
        }
        (self.bar - 1)
            .checked_mul(ts.ticks_per_bar())?
            .checked_add((self.beat - 1) * ts.ticks_per_beat())?
            .checked_add(self.tick)
    }
}

impl fmt::Display for MusicalPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{:03}", self.bar, self.beat, self.tick)
    }
}

/// The twelve pitch classes, spelled with sharps on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PitchClass {
    C,
    #[serde(rename = "C#")]
    #[ts(rename = "C#")]
    CSharp,
    D,
    #[serde(rename = "D#")]
    #[ts(rename = "D#")]
    DSharp,
    E,
    F,
    #[serde(rename = "F#")]
    #[ts(rename = "F#")]
    FSharp,
    G,
    #[serde(rename = "G#")]
    #[ts(rename = "G#")]
    GSharp,
    A,
    #[serde(rename = "A#")]
    #[ts(rename = "A#")]
    ASharp,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum KeyMode {
    Major,
    Minor,
}

/// A musical key signature (tonic + mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Key {
    pub tonic: PitchClass,
    pub mode: KeyMode,
}

impl Default for Key {
    fn default() -> Self {
        Self {
            tonic: PitchClass::C,
            mode: KeyMode::Major,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_bounds() {
        assert!(Tempo::new(120.0).is_some());
        assert!(Tempo::new(19.99).is_none());
        assert!(Tempo::new(1000.0).is_none());
        assert!(Tempo::new(f64::NAN).is_none());
        assert!(Tempo::new(f64::INFINITY).is_none());
    }

    #[test]
    fn tempo_converts_ticks_to_seconds() {
        let t = Tempo::new(120.0).unwrap();
        assert!((t.ticks_to_seconds(Ticks::from(PPQ)) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn time_signature_validation() {
        assert!(TimeSignature::new(4, 4).is_some());
        assert!(TimeSignature::new(7, 8).is_some());
        assert!(TimeSignature::new(0, 4).is_none());
        assert!(TimeSignature::new(4, 3).is_none());
        assert!(TimeSignature::new(33, 4).is_none());
    }

    #[test]
    fn ticks_per_bar_matches_metre() {
        assert_eq!(TimeSignature::new(4, 4).unwrap().ticks_per_bar(), 3840);
        assert_eq!(TimeSignature::new(6, 8).unwrap().ticks_per_bar(), 2880);
        assert_eq!(TimeSignature::new(3, 4).unwrap().ticks_per_beat(), 960);
    }

    #[test]
    fn position_round_trip() {
        let ts = TimeSignature::new(7, 8).unwrap();
        for ticks in [0, 1, 479, 480, 3359, 3360, 100_000] {
            let pos = MusicalPosition::from_ticks(ticks, ts);
            assert_eq!(pos.to_ticks(ts), Some(ticks), "{pos}");
        }
        assert_eq!(MusicalPosition::from_ticks(0, ts).to_string(), "1.1.000");
        assert_eq!(
            MusicalPosition {
                bar: 1,
                beat: 8,
                tick: 0
            }
            .to_ticks(ts),
            None
        );
    }
}
