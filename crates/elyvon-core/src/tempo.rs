//! Tempo map: conversions between musical time (ticks, bars) and audio time
//! (seconds, sample frames) with tempo and metre changes.
//!
//! * Tempo changes are **steps** at arbitrary tick positions.
//! * Metre changes happen on **bar lines**, addressed by zero-based bar index,
//!   so they always stay on a bar boundary when earlier bars change.
//!
//! Every conversion is a pure function of its input position computed from the
//! start of the containing segment. Nothing is accumulated block by block, so a
//! scheduler that asks "which sample is tick T?" gets the same answer whatever
//! its block size, and long renders do not drift. All methods are allocation-free
//! and may be called from the real-time thread once the map is built.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::music::{MusicalPosition, PPQ, Tempo, Ticks, TimeSignature};

/// A tempo step: from `tick` on, the song runs at `bpm`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TempoChange {
    #[ts(type = "number")]
    pub tick: Ticks,
    pub bpm: Tempo,
}

/// A metre change at the start of zero-based bar `bar` (bar 1 is index 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MeterChange {
    pub bar: u32,
    pub time_signature: TimeSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TempoMapError {
    TempoOrder,
    InvalidTempo,
    MeterOrder,
    InvalidMeter,
    TooManyChanges,
}

impl std::fmt::Display for TempoMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TempoOrder => {
                "tempo changes must be after the start and in strictly increasing order"
            }
            Self::InvalidTempo => "tempo must be between 20 and 999 BPM",
            Self::MeterOrder => {
                "metre changes must be after bar 1 and in strictly increasing bar order"
            }
            Self::InvalidMeter => "unsupported time signature",
            Self::TooManyChanges => "too many tempo or metre changes",
        })
    }
}

impl std::error::Error for TempoMapError {}

/// Upper bound on tempo or metre changes per project.
pub const MAX_CHANGES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq)]
struct TempoSeg {
    tick: Ticks,
    bpm: f64,
    /// Seconds from song start at `tick`.
    seconds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MeterSeg {
    bar: u64,
    tick: Ticks,
    ts: TimeSignature,
}

/// Bar containing a tick, with its metre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarInfo {
    /// Zero-based bar index.
    pub bar: u64,
    pub start: Ticks,
    pub time_signature: TimeSignature,
}

impl BarInfo {
    pub fn end(&self) -> Ticks {
        self.start + self.time_signature.ticks_per_bar()
    }
}

/// An immutable, validated tempo and metre map.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    tempo: Vec<TempoSeg>,
    meters: Vec<MeterSeg>,
}

impl TempoMap {
    /// A map with one tempo and one metre for the whole song.
    pub fn constant(tempo: Tempo, ts: TimeSignature) -> Self {
        Self {
            tempo: vec![TempoSeg {
                tick: 0,
                bpm: tempo.bpm(),
                seconds: 0.0,
            }],
            meters: vec![MeterSeg {
                bar: 0,
                tick: 0,
                ts,
            }],
        }
    }

    /// Builds a map from the initial tempo/metre plus sorted change lists.
    pub fn new(
        tempo: Tempo,
        tempo_changes: &[TempoChange],
        ts: TimeSignature,
        meter_changes: &[MeterChange],
    ) -> Result<Self, TempoMapError> {
        if tempo_changes.len() > MAX_CHANGES || meter_changes.len() > MAX_CHANGES {
            return Err(TempoMapError::TooManyChanges);
        }
        if !tempo.is_valid() {
            return Err(TempoMapError::InvalidTempo);
        }
        if !ts.is_valid() {
            return Err(TempoMapError::InvalidMeter);
        }
        let mut map = Self::constant(tempo, ts);
        let mut last_tick = 0;
        for change in tempo_changes {
            if change.tick <= last_tick {
                return Err(TempoMapError::TempoOrder);
            }
            if !change.bpm.is_valid() {
                return Err(TempoMapError::InvalidTempo);
            }
            let prev = map.tempo[map.tempo.len() - 1];
            map.tempo.push(TempoSeg {
                tick: change.tick,
                bpm: change.bpm.bpm(),
                seconds: prev.seconds + seg_seconds(change.tick - prev.tick, prev.bpm),
            });
            last_tick = change.tick;
        }
        let mut last_bar = 0;
        for change in meter_changes {
            if u64::from(change.bar) <= last_bar {
                return Err(TempoMapError::MeterOrder);
            }
            if !change.time_signature.is_valid() {
                return Err(TempoMapError::InvalidMeter);
            }
            let prev = map.meters[map.meters.len() - 1];
            let bar = u64::from(change.bar);
            map.meters.push(MeterSeg {
                bar,
                tick: prev.tick + (bar - prev.bar) * prev.ts.ticks_per_bar(),
                ts: change.time_signature,
            });
            last_bar = bar;
        }
        Ok(map)
    }

    /// Initial tempo in BPM.
    pub fn initial_bpm(&self) -> f64 {
        self.tempo[0].bpm
    }

    fn tempo_seg_for_tick(&self, tick: f64) -> &TempoSeg {
        let i = self.tempo.partition_point(|s| (s.tick as f64) <= tick);
        &self.tempo[i.saturating_sub(1)]
    }

    fn tempo_seg_for_seconds(&self, seconds: f64) -> &TempoSeg {
        let i = self.tempo.partition_point(|s| s.seconds <= seconds);
        &self.tempo[i.saturating_sub(1)]
    }

    /// Tempo in BPM at a tick.
    pub fn bpm_at(&self, tick: Ticks) -> f64 {
        self.tempo_seg_for_tick(tick as f64).bpm
    }

    /// Seconds from song start at a (possibly fractional) tick position.
    pub fn seconds_at(&self, tick: f64) -> f64 {
        let tick = tick.max(0.0);
        let seg = self.tempo_seg_for_tick(tick);
        seg.seconds + (tick - seg.tick as f64) / f64::from(PPQ) * 60.0 / seg.bpm
    }

    /// Fractional tick at a time in seconds.
    pub fn tick_at_seconds(&self, seconds: f64) -> f64 {
        let seconds = seconds.max(0.0);
        let seg = self.tempo_seg_for_seconds(seconds);
        seg.tick as f64 + (seconds - seg.seconds) * seg.bpm / 60.0 * f64::from(PPQ)
    }

    /// Exact (unrounded) sample position of a tick.
    pub fn samples_at(&self, tick: f64, sample_rate: u32) -> f64 {
        self.seconds_at(tick) * f64::from(sample_rate)
    }

    /// The sample frame on which an event at `tick` is scheduled
    /// (nearest frame, ties rounded up). This is the single rounding rule used
    /// by the engine, the offline renderer and the tests.
    pub fn sample_at_tick(&self, tick: Ticks, sample_rate: u32) -> u64 {
        round_frame(self.samples_at(tick as f64, sample_rate))
    }

    /// Fractional tick at a sample frame.
    pub fn tick_at_sample(&self, frame: u64, sample_rate: u32) -> f64 {
        self.tick_at_seconds(frame as f64 / f64::from(sample_rate))
    }

    fn meter_seg_for_tick(&self, tick: Ticks) -> &MeterSeg {
        let i = self.meters.partition_point(|m| m.tick <= tick);
        &self.meters[i.saturating_sub(1)]
    }

    fn meter_seg_for_bar(&self, bar: u64) -> &MeterSeg {
        let i = self.meters.partition_point(|m| m.bar <= bar);
        &self.meters[i.saturating_sub(1)]
    }

    /// Time signature in effect at a tick.
    pub fn time_signature_at(&self, tick: Ticks) -> TimeSignature {
        self.meter_seg_for_tick(tick).ts
    }

    /// Tick at which zero-based `bar` starts.
    pub fn bar_start(&self, bar: u64) -> Ticks {
        let m = self.meter_seg_for_bar(bar);
        m.tick
            .saturating_add((bar - m.bar).saturating_mul(m.ts.ticks_per_bar()))
    }

    /// The bar containing `tick`.
    pub fn bar_at(&self, tick: Ticks) -> BarInfo {
        let m = self.meter_seg_for_tick(tick);
        let per_bar = m.ts.ticks_per_bar();
        let n = (tick - m.tick) / per_bar;
        BarInfo {
            bar: m.bar + n,
            start: m.tick + n * per_bar,
            time_signature: m.ts,
        }
    }

    /// 1-based bar.beat.tick, honouring metre changes.
    pub fn position(&self, tick: Ticks) -> MusicalPosition {
        let bar = self.bar_at(tick);
        let per_beat = bar.time_signature.ticks_per_beat();
        let in_bar = tick - bar.start;
        MusicalPosition {
            bar: bar.bar + 1,
            beat: in_bar / per_beat + 1,
            tick: in_bar % per_beat,
        }
    }

    /// Inverse of [`position`](Self::position). `None` for out-of-range parts.
    pub fn tick_of(&self, pos: MusicalPosition) -> Option<Ticks> {
        if pos.bar == 0 || pos.beat == 0 {
            return None;
        }
        let start = self.bar_start(pos.bar - 1);
        let ts = self.meter_seg_for_bar(pos.bar - 1).ts;
        if pos.beat > u64::from(ts.numerator) || pos.tick >= ts.ticks_per_beat() {
            return None;
        }
        start
            .checked_add((pos.beat - 1) * ts.ticks_per_beat())?
            .checked_add(pos.tick)
    }

    /// Beats (metronome clicks) with `from <= tick < to`, without allocating.
    pub fn beats(&self, from: Ticks, to: Ticks) -> Beats<'_> {
        let bar = self.bar_at(from);
        let per_beat = bar.time_signature.ticks_per_beat();
        let k = (from - bar.start).div_ceil(per_beat);
        Beats {
            map: self,
            bar,
            next: bar.start + k * per_beat,
            to,
        }
    }

    /// Change lists as stored in a project (excluding the initial values).
    pub fn tempo_changes(&self) -> impl Iterator<Item = (Ticks, f64)> + '_ {
        self.tempo.iter().skip(1).map(|s| (s.tick, s.bpm))
    }
}

fn seg_seconds(ticks: Ticks, bpm: f64) -> f64 {
    ticks as f64 / f64::from(PPQ) * 60.0 / bpm
}

/// Rounds an exact sample position to its frame: nearest, ties up.
#[inline]
pub fn round_frame(samples: f64) -> u64 {
    if samples <= 0.0 {
        0
    } else {
        (samples + 0.5).floor() as u64
    }
}

/// A metronome beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Beat {
    pub tick: Ticks,
    /// First beat of a bar.
    pub downbeat: bool,
    /// Zero-based bar index.
    pub bar: u64,
}

/// Iterator returned by [`TempoMap::beats`].
#[derive(Debug, Clone)]
pub struct Beats<'a> {
    map: &'a TempoMap,
    bar: BarInfo,
    next: Ticks,
    to: Ticks,
}

impl Iterator for Beats<'_> {
    type Item = Beat;

    fn next(&mut self) -> Option<Beat> {
        if self.next >= self.bar.end() {
            self.bar = self.map.bar_at(self.bar.end());
            self.next = self.bar.start;
        }
        if self.next >= self.to {
            return None;
        }
        let beat = Beat {
            tick: self.next,
            downbeat: self.next == self.bar.start,
            bar: self.bar.bar,
        };
        self.next += self.bar.time_signature.ticks_per_beat();
        Some(beat)
    }
}

/// Tap-tempo estimator: median of the recent tap intervals.
#[derive(Debug, Clone, Default)]
pub struct TapTempo {
    taps: Vec<f64>,
}

impl TapTempo {
    /// Taps further apart than this start a new measurement.
    pub const RESET_AFTER_SECONDS: f64 = 2.0;
    const KEEP: usize = 8;

    /// Registers a tap at `seconds` (monotonic clock). Returns the estimated
    /// tempo once at least three taps were made, if it is a valid tempo.
    pub fn tap(&mut self, seconds: f64) -> Option<f64> {
        if let Some(last) = self.taps.last()
            && (seconds - last > Self::RESET_AFTER_SECONDS || seconds <= *last)
        {
            self.taps.clear();
        }
        self.taps.push(seconds);
        if self.taps.len() > Self::KEEP {
            self.taps.remove(0);
        }
        if self.taps.len() < 3 {
            return None;
        }
        let mut intervals: Vec<f64> = self.taps.windows(2).map(|w| w[1] - w[0]).collect();
        intervals.sort_by(f64::total_cmp);
        let mid = intervals.len() / 2;
        let median = if intervals.len().is_multiple_of(2) {
            (intervals[mid - 1] + intervals[mid]) / 2.0
        } else {
            intervals[mid]
        };
        let bpm = (60.0 / median * 100.0).round() / 100.0;
        Tempo::new(bpm).map(Tempo::bpm)
    }

    pub fn taps(&self) -> usize {
        self.taps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(n: u8, d: u8) -> TimeSignature {
        TimeSignature::new(n, d).unwrap()
    }

    fn bpm(v: f64) -> Tempo {
        Tempo::new(v).unwrap()
    }

    #[test]
    fn constant_tempo_conversions() {
        let map = TempoMap::constant(bpm(120.0), ts(4, 4));
        assert_eq!(map.seconds_at(960.0), 0.5);
        assert_eq!(map.sample_at_tick(960, 48_000), 24_000);
        assert_eq!(map.sample_at_tick(3840, 44_100), 88_200);
        assert!((map.tick_at_sample(24_000, 48_000) - 960.0).abs() < 1e-9);
        assert!((map.tick_at_seconds(0.5) - 960.0).abs() < 1e-9);
    }

    #[test]
    fn tempo_steps_are_exact_from_segment_starts() {
        // 120 BPM for 1 bar, then 60 BPM.
        let map = TempoMap::new(
            bpm(120.0),
            &[TempoChange {
                tick: 3840,
                bpm: bpm(60.0),
            }],
            ts(4, 4),
            &[],
        )
        .unwrap();
        assert_eq!(map.seconds_at(3840.0), 2.0);
        assert_eq!(map.seconds_at(3840.0 + 960.0), 3.0);
        assert_eq!(map.sample_at_tick(7680, 48_000), 6 * 48_000);
        assert!((map.tick_at_seconds(3.0) - 4800.0).abs() < 1e-9);
        assert_eq!(map.bpm_at(3839), 120.0);
        assert_eq!(map.bpm_at(3840), 60.0);
    }

    #[test]
    fn rejects_bad_changes() {
        let c = |tick, v| TempoChange { tick, bpm: bpm(v) };
        assert_eq!(
            TempoMap::new(bpm(120.0), &[c(0, 90.0)], ts(4, 4), &[]).unwrap_err(),
            TempoMapError::TempoOrder
        );
        assert_eq!(
            TempoMap::new(bpm(120.0), &[c(10, 90.0), c(10, 80.0)], ts(4, 4), &[]).unwrap_err(),
            TempoMapError::TempoOrder
        );
        let m = |bar, n, d| MeterChange {
            bar,
            time_signature: TimeSignature {
                numerator: n,
                denominator: d,
            },
        };
        assert_eq!(
            TempoMap::new(bpm(120.0), &[], ts(4, 4), &[m(0, 3, 4)]).unwrap_err(),
            TempoMapError::MeterOrder
        );
        assert_eq!(
            TempoMap::new(bpm(120.0), &[], ts(4, 4), &[m(2, 3, 5)]).unwrap_err(),
            TempoMapError::InvalidMeter
        );
    }

    #[test]
    fn metre_changes_place_bars() {
        // 2 bars of 4/4, 3 bars of 3/4, then 7/8.
        let map = TempoMap::new(
            bpm(100.0),
            &[],
            ts(4, 4),
            &[
                MeterChange {
                    bar: 2,
                    time_signature: ts(3, 4),
                },
                MeterChange {
                    bar: 5,
                    time_signature: ts(7, 8),
                },
            ],
        )
        .unwrap();
        assert_eq!(map.bar_start(2), 7680);
        assert_eq!(map.bar_start(5), 7680 + 3 * 2880);
        assert_eq!(map.bar_start(6), 7680 + 3 * 2880 + 3360);
        let b = map.bar_at(7680 + 2880 + 5);
        assert_eq!((b.bar, b.start, b.time_signature), (3, 10_560, ts(3, 4)));
        for tick in [0, 7679, 7680, 10_559, 16_320, 16_321, 20_000, 1_000_000] {
            let pos = map.position(tick);
            assert_eq!(map.tick_of(pos), Some(tick), "{pos}");
        }
        assert_eq!(map.position(16_320).to_string(), "6.1.000");
    }

    #[test]
    fn beats_follow_each_metre() {
        for (n, d, expected) in [(3, 4, 3), (6, 8, 6), (7, 8, 7), (4, 4, 4)] {
            let map = TempoMap::constant(bpm(90.0), ts(n, d));
            let per_bar = ts(n, d).ticks_per_bar();
            let beats: Vec<_> = map.beats(0, per_bar * 2).collect();
            assert_eq!(beats.len(), expected * 2, "{n}/{d}");
            assert!(beats[0].downbeat && beats[expected].downbeat);
            assert_eq!(beats.iter().filter(|b| b.downbeat).count(), 2);
        }
        // Starting mid-beat rounds up to the next beat.
        let map = TempoMap::constant(bpm(90.0), ts(4, 4));
        let first = map.beats(1, 10_000).next().unwrap();
        assert_eq!(first.tick, 960);
        assert!(!first.downbeat);
    }

    #[test]
    fn beats_cross_metre_changes() {
        let map = TempoMap::new(
            bpm(120.0),
            &[],
            ts(4, 4),
            &[MeterChange {
                bar: 1,
                time_signature: ts(7, 8),
            }],
        )
        .unwrap();
        let beats: Vec<_> = map.beats(0, 3840 + 3360).collect();
        assert_eq!(beats.len(), 4 + 7);
        assert_eq!(beats[4].tick, 3840);
        assert!(beats[4].downbeat);
        assert_eq!(beats[5].tick, 3840 + 480);
    }

    #[test]
    fn extreme_tempos_round_trip() {
        for v in [Tempo::MIN_BPM, 33.333, 999.0] {
            let map = TempoMap::constant(bpm(v), ts(4, 4));
            for tick in [0u64, 1, 959, 960, 1 << 30] {
                let s = map.seconds_at(tick as f64);
                assert!((map.tick_at_seconds(s) - tick as f64).abs() < 1e-6);
            }
            // One tick at 999 BPM is ~2.9 samples at 44.1 kHz: still distinct frames.
            let a = map.sample_at_tick(960, 44_100);
            let b = map.sample_at_tick(961, 44_100);
            assert!(b > a);
        }
    }

    #[test]
    fn ten_minute_tempo_map_has_exact_length() {
        // 4 minutes at 120, 3 at 90, 3 at 150 BPM.
        let t1 = 4 * 120 * 960;
        let t2 = t1 + 3 * 90 * 960;
        let end = t2 + 3 * 150 * 960;
        let map = TempoMap::new(
            bpm(120.0),
            &[
                TempoChange {
                    tick: t1,
                    bpm: bpm(90.0),
                },
                TempoChange {
                    tick: t2,
                    bpm: bpm(150.0),
                },
            ],
            ts(4, 4),
            &[],
        )
        .unwrap();
        assert_eq!(map.seconds_at(end as f64), 600.0);
        assert_eq!(map.sample_at_tick(end, 48_000), 600 * 48_000);
        assert_eq!(map.sample_at_tick(end, 44_100), 600 * 44_100);
    }

    #[test]
    fn tap_tempo_uses_median_and_resets() {
        let mut tap = TapTempo::default();
        assert_eq!(tap.tap(10.0), None);
        assert_eq!(tap.tap(10.5), None);
        assert_eq!(tap.tap(11.0), Some(120.0));
        // An outlier does not move the median much.
        assert_eq!(tap.tap(11.7), Some(120.0));
        assert_eq!(tap.tap(12.2), Some(120.0));
        // Long pause restarts.
        assert_eq!(tap.tap(20.0), None);
        assert_eq!(tap.taps(), 1);
        // Too slow for the valid range.
        let mut slow = TapTempo::default();
        slow.tap(0.0);
        slow.tap(1.9);
        assert_eq!(slow.tap(3.8), Some(31.58));
    }
}
