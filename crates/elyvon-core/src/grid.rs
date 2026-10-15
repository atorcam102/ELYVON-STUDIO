//! Snap grid: bar/beat/note divisions, triplets and swing, relative to bar
//! lines of a [`TempoMap`] (so it stays correct across metre changes).
//!
//! The UI mirrors this logic for drag previews; the shared golden vectors in
//! `crates/elyvon-core/tests/fixtures/grid_golden.json` keep both in sync.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::music::{PPQ, Ticks, TimeSignature};
use crate::tempo::TempoMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GridDivision {
    Bar,
    /// The beat of the current time signature (its denominator).
    Beat,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
}

/// Snap settings. `swing` is 0 (straight) … 100 (full triplet feel) and
/// delays every second grid point by `swing% × step / 3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GridSpec {
    pub division: GridDivision,
    pub triplet: bool,
    pub swing: u8,
}

impl Default for GridSpec {
    fn default() -> Self {
        Self {
            division: GridDivision::Sixteenth,
            triplet: false,
            swing: 0,
        }
    }
}

impl GridSpec {
    /// Grid step in ticks inside a bar of metre `ts`. `None` for whole bars.
    pub fn step(&self, ts: TimeSignature) -> Option<Ticks> {
        let base = match self.division {
            GridDivision::Bar => return None,
            GridDivision::Beat => ts.ticks_per_beat(),
            GridDivision::Half => Ticks::from(PPQ) * 2,
            GridDivision::Quarter => Ticks::from(PPQ),
            GridDivision::Eighth => Ticks::from(PPQ) / 2,
            GridDivision::Sixteenth => Ticks::from(PPQ) / 4,
            GridDivision::ThirtySecond => Ticks::from(PPQ) / 8,
        };
        // PPQ = 960 makes every triplet step an integer.
        Some(if self.triplet { base * 2 / 3 } else { base })
    }

    fn swing_offset(&self, step: Ticks, index: u64) -> Ticks {
        if self.triplet || self.swing == 0 || index.is_multiple_of(2) {
            0
        } else {
            step * Ticks::from(self.swing.min(100)) / 300
        }
    }

    /// Nearest grid point to `tick`.
    pub fn snap(&self, map: &TempoMap, tick: Ticks) -> Ticks {
        let bar = map.bar_at(tick);
        let Some(step) = self.step(bar.time_signature) else {
            let end = bar.end();
            return if tick - bar.start <= end - tick {
                bar.start
            } else {
                end
            };
        };
        let k = (tick - bar.start) / step;
        let mut best = bar.end();
        for i in k.saturating_sub(1)..=k + 1 {
            let point = (bar.start + i * step + self.swing_offset(step, i)).min(bar.end());
            if point.abs_diff(tick) < best.abs_diff(tick) {
                best = point;
            }
        }
        best
    }

    /// Visits grid points `from <= p < to` (bar lines included).
    pub fn points(&self, map: &TempoMap, from: Ticks, to: Ticks, mut visit: impl FnMut(Ticks)) {
        let mut bar = map.bar_at(from);
        while bar.start < to {
            match self.step(bar.time_signature) {
                None => {
                    if bar.start >= from {
                        visit(bar.start);
                    }
                }
                Some(step) => {
                    let mut i = 0;
                    loop {
                        let p = bar.start + i * step + self.swing_offset(step, i);
                        if p >= bar.end() || p >= to {
                            break;
                        }
                        if p >= from {
                            visit(p);
                        }
                        i += 1;
                    }
                }
            }
            bar = map.bar_at(bar.end());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::Tempo;
    use crate::tempo::MeterChange;

    fn spec(division: GridDivision, triplet: bool, swing: u8) -> GridSpec {
        GridSpec {
            division,
            triplet,
            swing,
        }
    }

    fn map44() -> TempoMap {
        TempoMap::constant(Tempo::DEFAULT, TimeSignature::default())
    }

    #[test]
    fn straight_divisions() {
        let m = map44();
        assert_eq!(spec(GridDivision::Bar, false, 0).snap(&m, 1919), 0);
        assert_eq!(spec(GridDivision::Bar, false, 0).snap(&m, 1921), 3840);
        assert_eq!(spec(GridDivision::Quarter, false, 0).snap(&m, 1500), 1920);
        assert_eq!(spec(GridDivision::Sixteenth, false, 0).snap(&m, 130), 240);
        assert_eq!(spec(GridDivision::Sixteenth, false, 0).snap(&m, 110), 0);
    }

    #[test]
    fn triplets() {
        let m = map44();
        let s = spec(GridDivision::Eighth, true, 0);
        assert_eq!(s.step(TimeSignature::default()), Some(320));
        assert_eq!(s.snap(&m, 300), 320);
        assert_eq!(s.snap(&m, 500), 640);
    }

    #[test]
    fn swing_delays_off_beats() {
        let m = map44();
        let s = spec(GridDivision::Eighth, false, 100);
        // Off-beat eighth moves from 480 to 480 + 160 = 640 (triplet feel).
        let mut pts = Vec::new();
        s.points(&m, 0, 1920, |p| pts.push(p));
        assert_eq!(pts, [0, 640, 960, 1600]);
        assert_eq!(s.snap(&m, 600), 640);
        assert_eq!(s.snap(&m, 400), 640);
        let half = spec(GridDivision::Sixteenth, false, 50);
        let mut pts = Vec::new();
        half.points(&m, 0, 480, |p| pts.push(p));
        assert_eq!(pts, [0, 280]);
    }

    #[test]
    fn grid_restarts_at_each_bar_in_odd_metres() {
        // 7/8 bar has 3360 ticks: quarter grid 0, 960, 1920, 2880 then bar 3360.
        let m = TempoMap::new(
            Tempo::DEFAULT,
            &[],
            TimeSignature::new(7, 8).unwrap(),
            &[MeterChange {
                bar: 1,
                time_signature: TimeSignature::new(3, 4).unwrap(),
            }],
        )
        .unwrap();
        let mut pts = Vec::new();
        spec(GridDivision::Quarter, false, 0).points(&m, 0, 3360 + 2880, |p| pts.push(p));
        assert_eq!(pts, [0, 960, 1920, 2880, 3360, 4320, 5280]);
        assert_eq!(spec(GridDivision::Quarter, false, 0).snap(&m, 3300), 3360);
        assert_eq!(spec(GridDivision::Beat, false, 0).snap(&m, 3100), 2880);
    }
}
