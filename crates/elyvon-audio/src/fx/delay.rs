//! Stereo delay: free or tempo-synchronised times (following the tempo map
//! at the playhead), ping-pong, low/high cut in the feedback loop and a tape
//! character with wow, flutter and saturation.
//!
//! Feedback is protected: the loop gain never exceeds 0.99 and every pass
//! goes through a soft saturator that bounds its level, so even 100 %
//! feedback rings at a bounded level instead of growing without limit.

use elyvon_project::effects::{EffectKind, ParamValues, division_quarters};

use super::filters::OnePole;
use super::{Bank, FxContext, Ramp, db};

/// Longest delay time.
pub const MAX_SECONDS: f64 = 4.0;
/// Highest loop gain at 100 % feedback.
pub const MAX_LOOP_GAIN: f32 = 0.99;

mod dp {
    pub const SYNC: usize = 0;
    pub const DIV_L: usize = 1;
    pub const DIV_R: usize = 2;
    pub const TIME_L: usize = 3;
    pub const TIME_R: usize = 4;
    pub const FEEDBACK: usize = 5;
    pub const PING_PONG: usize = 6;
    pub const LOW_CUT: usize = 7;
    pub const HIGH_CUT: usize = 8;
    pub const MODE: usize = 9;
    pub const WOW: usize = 10;
    pub const SATURATION: usize = 11;
    pub const MIX: usize = 12;
    pub const WIDTH: usize = 13;
    pub const MONO: usize = 14;
    pub const OUTPUT: usize = 15;
}

/// Dry and wet gains for a mix of `m` (0…1): the dry signal stays at unity
/// up to 50 % and the wet signal reaches unity at 50 %.
#[inline]
pub fn balanced_mix(m: f32) -> (f32, f32) {
    ((2.0 * (1.0 - m)).min(1.0), (2.0 * m).min(1.0))
}

/// Soft limiter that leaves |x| ≤ 1 untouched and bounds the output at 2.
#[inline]
pub fn protect(x: f32) -> f32 {
    let a = x.abs();
    if a <= 1.0 {
        x
    } else {
        x.signum() * (1.0 + (a - 1.0).tanh())
    }
}

/// A stereo circular buffer with fractional (cubic Hermite) reads.
#[derive(Debug, Clone)]
pub struct Line {
    data: Vec<[f32; 2]>,
    at: usize,
}

impl Line {
    pub fn new(frames: usize) -> Self {
        Self {
            data: vec![[0.0; 2]; frames.max(4)],
            at: 0,
        }
    }

    #[inline]
    pub fn write(&mut self, v: [f32; 2]) {
        self.data[self.at] = v;
        self.at += 1;
        if self.at == self.data.len() {
            self.at = 0;
        }
    }

    #[inline]
    fn tap(&self, back: usize, ch: usize) -> f32 {
        let n = self.data.len();
        self.data[(self.at + n - (back % n)) % n][ch]
    }

    /// Value written `delay` frames ago (≥ 2), Hermite-interpolated.
    #[inline]
    pub fn read(&self, delay: f32, ch: usize) -> f32 {
        let max = (self.data.len() - 3) as f32;
        let d = delay.clamp(2.0, max);
        let i = d.floor();
        let t = d - i;
        let i = i as usize;
        let (xm1, x0, x1, x2) = (
            self.tap(i - 1, ch),
            self.tap(i, ch),
            self.tap(i + 1, ch),
            self.tap(i + 2, ch),
        );
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }

    /// Linear read (gain ≤ 1 at every frequency: safe in recursive loops).
    #[inline]
    pub fn read_linear(&self, delay: f32, ch: usize) -> f32 {
        let max = (self.data.len() - 2) as f32;
        let d = delay.clamp(1.0, max);
        let i = d.floor();
        let t = d - i;
        let a = self.tap(i as usize, ch);
        let b = self.tap(i as usize + 1, ch);
        a + (b - a) * t
    }

    pub fn adopt(&mut self, old: &mut Line) {
        if old.data.len() == self.data.len() {
            std::mem::swap(&mut self.data, &mut old.data);
            self.at = old.at;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Delay {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    line: Line,
    /// Current delay per channel, frames (glides to the target). Double
    /// precision: an f32 glide stalls several frames short at long delays.
    time: [f64; 2],
    lp: [OnePole; 2],
    hp: [OnePole; 2],
    filters: (f64, f64),
    wow: f64,
    flutter: f64,
    gains: [Ramp; 4],
    /// The first chunk starts on the target time (no glide from the
    /// construction tempo).
    started: bool,
}

impl Delay {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        let frames = (MAX_SECONDS * 1.02 * f64::from(rate)) as usize + 8;
        let mut d = Self {
            bank: Bank::new(EffectKind::Delay, p, rate),
            out_peak: 0.0,
            rate,
            line: Line::new(frames),
            time: [0.0; 2],
            lp: [OnePole::default(); 2],
            hp: [OnePole::default(); 2],
            filters: (0.0, 0.0),
            wow: 0.0,
            flutter: 0.0,
            gains: [Ramp::hold(0.0); 4],
            started: false,
        };
        d.time = [
            d.target(dp::DIV_L, dp::TIME_L, 120.0),
            d.target(dp::DIV_R, dp::TIME_R, 120.0),
        ];
        d.gains = d.levels().map(Ramp::hold);
        d
    }

    fn target(&self, div: usize, time: usize, bpm: f64) -> f64 {
        let b = &self.bank;
        let seconds = if b.get(dp::SYNC) >= 0.5 {
            division_quarters(b.get(div).round() as usize) * 60.0 / bpm.max(1.0)
        } else {
            b.get(time) / 1000.0
        };
        (seconds.clamp(0.0, MAX_SECONDS) * f64::from(self.rate)).max(2.0)
    }

    /// Dry, wet, feedback and output gains.
    fn levels(&self) -> [f32; 4] {
        let b = &self.bank;
        let (dry, wet) = balanced_mix((b.get(dp::MIX) / 100.0) as f32);
        [
            dry,
            wet,
            (b.get(dp::FEEDBACK) / 100.0) as f32 * MAX_LOOP_GAIN,
            db(b.get(dp::OUTPUT)) as f32,
        ]
    }

    pub fn adopt(&mut self, old: &mut Delay) {
        self.bank.adopt(&old.bank);
        self.line.adopt(&mut old.line);
        self.time = old.time;
        self.lp = old.lp;
        self.hp = old.hp;
        self.wow = old.wow;
        self.flutter = old.flutter;
        self.gains = old.gains;
        self.started = old.started;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let rate = f64::from(self.rate);
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let bpm = cx.bpm_at(ck.start);
            let target = [
                self.target(dp::DIV_L, dp::TIME_L, bpm),
                self.target(dp::DIV_R, dp::TIME_R, bpm),
            ];
            if !self.started {
                self.time = target;
                self.started = true;
            }
            let b = &self.bank;
            let (lo, hi) = (b.get(dp::LOW_CUT), b.get(dp::HIGH_CUT));
            if (lo, hi) != self.filters {
                for ch in 0..2 {
                    self.hp[ch].set_freq(lo, self.rate);
                    self.lp[ch].set_freq(hi, self.rate);
                }
                self.filters = (lo, hi);
            }
            let tape = b.get(dp::MODE) >= 0.5;
            let ping = b.get(dp::PING_PONG) >= 0.5;
            let mono = b.get(dp::MONO) >= 0.5;
            let wow_depth = if tape { b.get(dp::WOW) / 100.0 } else { 0.0 };
            let sat = if tape {
                (b.get(dp::SATURATION) / 100.0) as f32
            } else {
                0.0
            };
            let drive = 1.0 + 4.0 * sat;
            let width = (b.get(dp::WIDTH) / 100.0) as f32;
            // Times glide: ~60 ms digitally, a slower tape-like 250 ms.
            let glide = 1.0 - (-1.0 / ((if tape { 0.25 } else { 0.06 }) * rate)).exp();
            let to = self.levels();
            let ramps: [Ramp; 4] = std::array::from_fn(|i| self.gains[i].step(fresh, to[i]));
            let tau = std::f64::consts::TAU;
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let j = ck.idx(i);
                let (dry, wet_g, fb, out) = (
                    ramps[0].at(j, ck.len),
                    ramps[1].at(j, ck.len),
                    ramps[2].at(j, ck.len),
                    ramps[3].at(j, ck.len),
                );
                // Wow (0.6 Hz) and flutter (7 Hz) modulate the read position.
                self.wow = (self.wow + 0.6 / rate).fract();
                self.flutter = (self.flutter + 7.1 / rate).fract();
                let modulation = (wow_depth
                    * (0.004 * (tau * self.wow).sin() + 0.000_35 * (tau * self.flutter).sin())
                    * rate) as f32;
                let mut wet = [0.0f32; 2];
                for ch in 0..2 {
                    self.time[ch] += (target[ch] - self.time[ch]) * glide;
                    let phase = if ch == 0 { 0.0 } else { 0.37 * modulation };
                    let v = self
                        .line
                        .read(self.time[ch] as f32 + modulation + phase, ch);
                    let v = self.lp[ch].lp(v);
                    wet[ch] = self.hp[ch].hp(v);
                }
                let (il, ir) = if mono || ping {
                    let m = 0.5 * (f[0] + f[1]);
                    (m, if ping { 0.0 } else { m })
                } else {
                    (f[0], f[1])
                };
                let (fl, fr) = if ping {
                    (wet[1], wet[0])
                } else {
                    (wet[0], wet[1])
                };
                // Saturation keeps unity small-signal gain (so the loop gain
                // stays below one) and rounds off the peaks.
                let shape = |x: f32| protect(x + sat * ((drive * x).tanh() / drive - x));
                self.line.write([shape(il + fl * fb), shape(ir + fr * fb)]);
                let m = 0.5 * (wet[0] + wet[1]);
                let s = 0.5 * (wet[0] - wet[1]) * width;
                f[0] = (f[0] * dry + (m + s) * wet_g) * out;
                f[1] = (f[1] * dry + (m - s) * wet_g) * out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Effect;
    use super::super::testing::*;
    use super::*;
    use elyvon_core::tempo::{TempoChange, TempoMap};
    use elyvon_core::{PPQ, Tempo, TimeSignature};

    fn impulse_response(
        s: &elyvon_project::effects::EffectSlot,
        rate: u32,
        frames: usize,
        map: &TempoMap,
        at: usize,
    ) -> Vec<f32> {
        let mut x = vec![0.0f32; frames * 2];
        x[at * 2] = 1.0;
        x[at * 2 + 1] = 1.0;
        run(&mut Effect::new(s, rate), &x, rate, 128, map)
    }

    fn first_echo(out: &[f32], after: usize) -> usize {
        out.chunks(2)
            .enumerate()
            .skip(after + 10)
            .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
            .map_or(0, |(i, _)| i)
    }

    #[test]
    fn synchronised_times_follow_the_tempo() {
        let rate = 48_000;
        let s = slot(
            EffectKind::Delay,
            &[
                ("divisionLeft", 9.0),
                ("divisionRight", 9.0),
                ("feedback", 0.0),
                ("mix", 100.0),
            ],
        );
        for bpm in [60.0, 120.0, 150.0] {
            let m = map(bpm);
            let out = impulse_response(&s, rate, rate as usize * 2, &m, 0);
            let expected = (60.0 / bpm * f64::from(rate)).round() as usize;
            assert!(
                first_echo(&out, 0).abs_diff(expected) <= 2,
                "{bpm}: {} vs {expected}",
                first_echo(&out, 0)
            );
        }
    }

    #[test]
    fn synchronised_times_follow_tempo_map_changes() {
        // 120 BPM for two bars, then 60 BPM: a quarter-note echo becomes 1 s.
        let rate = 48_000;
        let map = TempoMap::new(
            Tempo::new(120.0).expect("tempo"),
            &[TempoChange {
                tick: u64::from(PPQ) * 8,
                bpm: Tempo::new(60.0).expect("tempo"),
            }],
            TimeSignature::default(),
            &[],
        )
        .expect("map");
        let s = slot(
            EffectKind::Delay,
            &[
                ("divisionLeft", 9.0),
                ("divisionRight", 9.0),
                ("feedback", 0.0),
                ("mix", 100.0),
            ],
        );
        let early = impulse_response(&s, rate, rate as usize * 2, &map, 0);
        assert!(first_echo(&early, 0).abs_diff(24_000) <= 2);
        // The change is at 4 s; an impulse at 6 s echoes 1 s later.
        let at = 6 * rate as usize;
        let late = impulse_response(&s, rate, at + 2 * rate as usize, &map, at);
        assert!(
            first_echo(&late, at).abs_diff(at + 48_000) <= 4,
            "{}",
            first_echo(&late, at)
        );
    }

    #[test]
    fn full_feedback_stays_bounded() {
        let rate = 48_000;
        let m = map(120.0);
        for mode in [0.0, 1.0] {
            let s = slot(
                EffectKind::Delay,
                &[
                    ("feedback", 100.0),
                    ("mix", 100.0),
                    ("sync", 0.0),
                    ("timeLeftMs", 20.0),
                    ("timeRightMs", 31.0),
                    ("mode", mode),
                    ("lowCutHz", 20.0),
                    ("highCutHz", 20_000.0),
                ],
            );
            let mut x = sine(200.0, rate, 10 * rate as usize, 1.0);
            // Loud input for 1 s, then silence: the loop must not explode.
            for v in &mut x[2 * rate as usize..] {
                *v = 0.0;
            }
            let out = run(&mut Effect::new(&s, rate), &x, rate, 64, &m);
            let peak = out.iter().fold(0f32, |a, v| a.max(v.abs()));
            assert!(peak < 4.0, "mode {mode}: {peak}");
            assert!(out.iter().all(|v| v.is_finite()));
            // And it decays: the loop gain is below one.
            let tail = rms(&out[out.len() - 9_600..]);
            let early = rms(&out[2 * rate as usize * 2..(2 * rate as usize + 4_800) * 2]);
            assert!(tail < early, "mode {mode}: {tail} vs {early}");
        }
    }

    #[test]
    fn ping_pong_alternates_channels() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::Delay,
            &[
                ("sync", 0.0),
                ("timeLeftMs", 100.0),
                ("timeRightMs", 100.0),
                ("pingPong", 1.0),
                ("feedback", 50.0),
                ("mix", 100.0),
                ("lowCutHz", 20.0),
                ("highCutHz", 20_000.0),
            ],
        );
        let out = impulse_response(&s, rate, rate as usize, &m, 0);
        let at = |i: usize| (out[i * 2].abs(), out[i * 2 + 1].abs());
        let (l1, r1) = at(4_800);
        let (l2, r2) = at(9_600);
        assert!(l1 > 0.2 && r1 < 0.05, "{l1} {r1}");
        assert!(r2 > 0.1 && l2 < 0.05, "{l2} {r2}");
    }

    #[test]
    fn hermite_read_is_exact_on_integer_delays() {
        let mut l = Line::new(64);
        for i in 0..64 {
            l.write([i as f32, -(i as f32)]);
        }
        assert_eq!(l.read(2.0, 0), 62.0);
        assert_eq!(l.read(10.0, 1), -54.0);
        assert!((l.read(10.5, 0) - 53.5).abs() < 1e-5);
    }
}
