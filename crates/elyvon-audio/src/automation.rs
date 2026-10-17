//! Sample-accurate automation for the real-time engine (Phase 10).
//!
//! The control thread compiles every automated parameter of a strip into an
//! [`Envelope`]: tick-domain pieces (see `elyvon_project::automation`) split
//! at tempo changes and mapped to frames. Inside a piece the musical position
//! advances linearly with the frame, so evaluating a curve at frame `t` is
//! exact in ticks under any tempo map. Discontinuities (steps, jumps, region
//! lane edges) are crossed with a 2 ms linear ramp that ends on the new value
//! at the point's frame, so output stays a pure function of the playhead:
//! renders do not depend on the block size and live playback equals offline
//! export.
//!
//! Live control gestures (Touch/Latch/Write) reach the callback as
//! [`ParamOverride`]s through an SPSC queue; a touched parameter ramps to the
//! control value within one block, and a released Touch control returns to
//! the lane over 200 ms. Nothing here allocates after construction.

use elyvon_core::TempoMap;
use elyvon_core::tempo::round_frame;
use elyvon_project::automation::{
    AutomationMode, AutomationParam, Curve, InsertParam, ParamSpec, Scale, TickPiece,
};
use elyvon_project::automation_record::TouchPhase;

/// De-click ramp across discontinuities.
pub const STEP_RAMP_SECONDS: f64 = 0.002;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    Hold,
    Linear,
    Bezier(f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Piece {
    start: u64,
    /// Exclusive; `u64::MAX` for the last piece.
    end: u64,
    tick0: f64,
    ticks_per_frame: f64,
    t0: f64,
    t1: f64,
    v0: f64,
    v1: f64,
    shape: Shape,
    /// De-click ramp this piece takes part in: the value ramps to `next`
    /// over the `ramp` frames before `ramp_end` (a discontinuity at the end
    /// of this or a following piece). `ramp` is 0 when there is none.
    next: f64,
    ramp: u64,
    ramp_end: u64,
}

impl Piece {
    #[inline]
    fn base(&self, t: u64) -> f64 {
        if self.shape == Shape::Hold || self.t1 <= self.t0 {
            return self.v0;
        }
        let tick = self.tick0 + (t - self.start) as f64 * self.ticks_per_frame;
        let x = ((tick - self.t0) / (self.t1 - self.t0)).clamp(0.0, 1.0);
        let f = match self.shape {
            Shape::Linear => x,
            Shape::Bezier(c1, c2) => elyvon_project::automation::bezier(c1, c2, x),
            Shape::Hold => 0.0,
        };
        self.v0 + (self.v1 - self.v0) * f
    }

    #[inline]
    fn value(&self, t: u64, gain: Option<(f64, f64)>) -> f64 {
        let v = self.base(t);
        if self.ramp > 0 && t + self.ramp >= self.ramp_end {
            let w = ((t + 1 + self.ramp - self.ramp_end) as f64 / self.ramp as f64).min(1.0);
            match gain {
                // Level parameters ramp linearly in gain, not in dB, so a
                // jump from silence does not bunch up at the end of the ramp.
                Some((lo, hi)) => {
                    let to_gain = |n: f64| {
                        let db = lo + n * (hi - lo);
                        if db <= lo { 0.0 } else { 10f64.powf(db / 20.0) }
                    };
                    let g = to_gain(v) + (to_gain(self.next) - to_gain(v)) * w;
                    let db = if g <= 10f64.powf(lo / 20.0) {
                        lo
                    } else {
                        20.0 * g.log10()
                    };
                    ((db - lo) / (hi - lo)).clamp(0.0, 1.0)
                }
                None => v + (self.next - v) * w,
            }
        } else {
            v
        }
    }

    /// Value approached at the end (without the ramp).
    fn end_value(&self) -> f64 {
        if self.end == u64::MAX {
            return self.v1;
        }
        self.base(self.end.saturating_sub(1).max(self.start))
    }
}

/// A compiled lane (normalised values over engine frames).
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pieces: Vec<Piece>,
    /// dB range of a level parameter (de-click ramps run in linear gain).
    gain: Option<(f64, f64)>,
}

impl Envelope {
    /// Compiles tick pieces covering `[0, ∞)` for `map` at `rate`.
    pub fn compile(pieces: &[TickPiece], map: &TempoMap, rate: u32) -> Self {
        let changes: Vec<f64> = map.tempo_changes().map(|(t, _)| t as f64).collect();
        let frame = |tick: f64| -> u64 {
            if tick.is_infinite() {
                u64::MAX
            } else {
                round_frame(map.samples_at(tick.max(0.0), rate))
            }
        };
        let mut out: Vec<Piece> = Vec::with_capacity(pieces.len() + changes.len());
        for p in pieces {
            let mut cuts = vec![p.from];
            cuts.extend(changes.iter().copied().filter(|c| *c > p.from && *c < p.to));
            cuts.push(p.to);
            for w in cuts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let start = frame(a);
                let end = frame(b);
                if end <= start {
                    continue;
                }
                let bpm = map.bpm_at(a.max(0.0).floor() as u64);
                let shape = match p.curve {
                    Curve::Step => Shape::Hold,
                    Curve::Linear => Shape::Linear,
                    Curve::Bezier { c1, c2 } => Shape::Bezier(c1, c2),
                };
                out.push(Piece {
                    start,
                    end,
                    tick0: map.tick_at_sample(start, rate),
                    ticks_per_frame: bpm / 60.0 * f64::from(elyvon_core::PPQ) / f64::from(rate),
                    t0: p.t0,
                    t1: p.t1,
                    v0: p.v0,
                    v1: p.v1,
                    shape,
                    next: 0.0,
                    ramp: 0,
                    ramp_end: 0,
                });
            }
        }
        out.sort_by_key(|p| p.start);
        let ramp = (STEP_RAMP_SECONDS * f64::from(rate)).round().max(1.0) as u64;
        // Ramp into every discontinuity over the frames before it, across
        // pieces split by tempo changes, never back past the previous one.
        let mut run_start = out.first().map_or(0, |p| p.start);
        for i in 0..out.len() {
            let Some(n) = out.get(i + 1).copied() else {
                break;
            };
            let next = n.base(n.start);
            let end = out[i].end;
            if (out[i].end_value() - next).abs() <= 1e-9 {
                continue;
            }
            let r = ramp.min(end - run_start);
            for p in out[..=i].iter_mut().rev() {
                if p.end + r <= end {
                    break;
                }
                p.ramp = r;
                p.ramp_end = end;
                p.next = next;
            }
            run_start = end;
        }
        Self {
            pieces: out,
            gain: None,
        }
    }

    /// Marks the envelope as a level in dB over `[min_db, max_db]`
    /// (`min_db` is silence): de-click ramps become linear in gain.
    pub fn level(mut self, min_db: f64, max_db: f64) -> Self {
        self.gain = Some((min_db, max_db));
        self
    }

    fn index(&self, t: u64) -> usize {
        self.pieces
            .partition_point(|p| p.end <= t)
            .min(self.pieces.len().saturating_sub(1))
    }

    /// Normalised value at frame `t`.
    pub fn value(&self, t: u64) -> f64 {
        match self.pieces.get(self.index(t)) {
            Some(p) if t >= p.start => p.value(t, self.gain),
            Some(p) => p.v0,
            None => 0.0,
        }
    }

    /// Normalised values for frames `start..start + out.len()`.
    #[inline]
    pub fn fill(&self, start: u64, out: &mut [f32]) {
        if self.pieces.is_empty() {
            out.fill(0.0);
            return;
        }
        let mut i = self.index(start);
        for (k, o) in out.iter_mut().enumerate() {
            let t = start + k as u64;
            while self.pieces[i].end <= t && i + 1 < self.pieces.len() {
                i += 1;
            }
            let p = &self.pieces[i];
            *o = if t < p.start {
                p.v0
            } else {
                p.value(t, self.gain)
            } as f32;
        }
    }

    /// The value when it does not change over `[start, start + len)`.
    pub fn constant_over(&self, start: u64, len: usize) -> Option<f64> {
        let i = self.index(start);
        let p = self.pieces.get(i)?;
        let last = start + len as u64;
        let flat = p.shape == Shape::Hold || p.v0 == p.v1 || p.t1 <= p.t0;
        (start >= p.start && last <= p.end && flat && (p.ramp == 0 || last + p.ramp <= p.ramp_end))
            .then_some(p.v0)
    }

    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }
}

/// Native value of a normalised one, cheaply (real-time safe).
#[inline]
pub fn native(spec: &ParamSpec, n: f32) -> f32 {
    let n = n.clamp(0.0, 1.0);
    match spec.scale {
        Scale::Linear => spec.min as f32 + n * (spec.max - spec.min) as f32,
        Scale::Log => spec.min as f32 * ((spec.max / spec.min) as f32).powf(n),
    }
}

/// What a slot drives on its strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Volume,
    Pan,
    Mute,
    /// Index into the strip's sends.
    Send(usize),
    /// Index into the strip's inserts.
    Insert(usize, InsertParam),
}

/// Which strip an override addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripKey {
    Track(u128),
    Aux(u128),
    Master,
}

/// A live control value for the callback (Touch/Latch/Write capture).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamOverride {
    pub strip: StripKey,
    pub param: AutomationParam,
    /// In the parameter's unit.
    pub value: f64,
    pub phase: TouchPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Override {
    pub active: bool,
    /// Target (normalised).
    pub value: f32,
    /// Last applied value (ramp start).
    pub current: f32,
    /// Frames left of a Touch release back to the lane.
    pub release_left: u64,
    pub release_total: u64,
}

/// One automatable parameter of a strip.
#[derive(Debug, Clone)]
pub struct Slot {
    pub target: Target,
    pub param: AutomationParam,
    pub spec: ParamSpec,
    pub env: Option<Envelope>,
    pub static_norm: f32,
    pub over: Override,
    /// Native values of the last rendered span.
    pub buf: Vec<f32>,
    /// `Some(v)` when the last span was constant at `v` (native).
    pub constant: Option<f32>,
    /// The last span ignored the lane (Write mode while playing).
    ignore_env: bool,
}

impl Slot {
    pub fn new(
        target: Target,
        param: AutomationParam,
        env: Option<Envelope>,
        static_value: f64,
        chunk: usize,
    ) -> Self {
        let spec = param.spec();
        Self {
            target,
            param,
            spec,
            env,
            static_norm: spec.to_norm(static_value) as f32,
            over: Override::default(),
            buf: vec![0.0; chunk],
            constant: None,
            ignore_env: false,
        }
    }

    /// Whether the slot can change the signal compared to the static value.
    pub fn is_dynamic(&self) -> bool {
        self.env.is_some() || self.over.active || self.over.release_left > 0
    }

    /// Renders native values for `[pos, pos + len)` into `buf`. `ignore_env`
    /// (Write mode while playing) uses the static value under overrides;
    /// `advancing` false evaluates `pos` for the whole span.
    pub fn render(&mut self, pos: u64, len: usize, advancing: bool, ignore_env: bool) {
        let len = len.min(self.buf.len());
        self.ignore_env = ignore_env;
        let out = &mut self.buf[..len];
        let env = if ignore_env { None } else { self.env.as_ref() };
        // Base (lane or static), normalised.
        let mut constant = match env {
            None => Some(self.static_norm),
            Some(e) if !advancing => Some(e.value(pos) as f32),
            Some(e) => e.constant_over(pos, len).map(|v| v as f32),
        };
        match (constant, env) {
            (Some(c), _) => out.fill(c),
            (None, Some(e)) => e.fill(pos, out),
            (None, None) => out.fill(self.static_norm),
        }
        let o = &mut self.over;
        if o.active {
            // Ramp from the last applied value to the control within the span.
            if o.current != o.value {
                let n = len.max(1) as f32;
                for (i, v) in out.iter_mut().enumerate() {
                    *v = o.current + (o.value - o.current) * (i + 1) as f32 / n;
                }
                constant = None;
            } else {
                out.fill(o.value);
                constant = Some(o.value);
            }
            o.current = o.value;
        } else if o.release_left > 0 {
            let total = o.release_total.max(1) as f32;
            for v in out.iter_mut() {
                let w = 1.0 - o.release_left as f32 / total;
                *v = o.value + (*v - o.value) * w;
                if advancing {
                    o.release_left = o.release_left.saturating_sub(1);
                }
            }
            o.current = out.last().copied().unwrap_or(o.value);
            constant = None;
        } else {
            o.current = out.last().copied().unwrap_or(self.static_norm);
        }
        let spec = self.spec;
        match constant {
            Some(c) => {
                let v = native(&spec, c);
                out.fill(v);
                self.constant = Some(v);
            }
            None => {
                for v in out.iter_mut() {
                    *v = native(&spec, *v);
                }
                self.constant = None;
            }
        }
    }

    /// Native value at timeline frame `frame`, which is buffer entry
    /// `index` of the last span. Pure lane values are evaluated directly, so
    /// a value needed for a frame outside the span stays exact.
    pub fn value_at(&self, frame: u64, index: usize) -> f32 {
        if !self.over.active
            && self.over.release_left == 0
            && !self.ignore_env
            && let Some(e) = &self.env
        {
            return native(&self.spec, e.value(frame) as f32);
        }
        self.buf
            .get(index)
            .copied()
            .unwrap_or_else(|| native(&self.spec, self.static_norm))
    }

    /// Applies a control gesture. `release` is the Touch release in frames.
    pub fn apply(&mut self, value: f64, phase: TouchPhase, mode: AutomationMode, release: u64) {
        let n = self.spec.to_norm(value) as f32;
        let o = &mut self.over;
        if !o.active {
            o.active = true;
            o.release_left = 0;
        }
        o.value = n;
        if phase == TouchPhase::End && mode == AutomationMode::Touch {
            o.active = false;
            o.release_left = release;
            o.release_total = release;
        }
    }

    pub fn clear_override(&mut self) {
        self.over = Override::default();
    }
}

/// Automation state of one strip.
#[derive(Debug, Clone)]
pub struct StripAuto {
    pub mode: AutomationMode,
    pub slots: Vec<Slot>,
    pub volume: Option<usize>,
    pub pan: Option<usize>,
    pub mute: Option<usize>,
    /// Slot of each send (aligned with the strip's sends).
    pub sends: Vec<Option<usize>>,
    /// Static volume gain and pan law pair when not automated.
    pub static_gain: f32,
    pub static_pan: (f32, f32),
    pub mono: bool,
    pub law: elyvon_project::mixer::PanLaw,
}

impl StripAuto {
    pub fn slot_mut(&mut self, param: &AutomationParam) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.param == *param)
    }

    pub fn any_dynamic(&self) -> bool {
        self.slots.iter().any(Slot::is_dynamic)
    }

    /// Whether the fader stage needs per-sample processing.
    pub fn fader_dynamic(&self) -> bool {
        [self.volume, self.pan, self.mute]
            .into_iter()
            .flatten()
            .any(|i| self.slots[i].is_dynamic())
    }

    pub fn has_insert_slots(&self) -> bool {
        self.slots
            .iter()
            .any(|s| matches!(s.target, Target::Insert(..)) && s.is_dynamic())
    }

    /// Renders every slot for the span.
    pub fn render(&mut self, pos: u64, len: usize, advancing: bool, playing: bool) {
        let ignore = self.mode == AutomationMode::Write && playing;
        for s in &mut self.slots {
            s.render(pos, len, advancing, ignore);
        }
    }

    pub fn clear_overrides(&mut self) {
        for s in &mut self.slots {
            s.clear_override();
        }
    }

    /// Keeps running overrides of the same parameters from the old graph.
    pub fn adopt(&mut self, old: &StripAuto) {
        for s in &mut self.slots {
            if let Some(o) = old.slots.iter().find(|o| o.param == s.param) {
                s.over = o.over;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use elyvon_core::{Tempo, TempoChange, TimeSignature};
    use elyvon_project::automation::{AutomationLane, AutomationPoint, lane_pieces};

    fn lane(points: &[(u64, f64, Curve)]) -> AutomationLane {
        AutomationLane {
            param: AutomationParam::Pan,
            points: points
                .iter()
                .enumerate()
                .map(|(i, &(tick, value, curve))| AutomationPoint {
                    id: i as u32,
                    tick,
                    value,
                    curve,
                })
                .collect(),
        }
    }

    #[test]
    fn linear_ramps_are_exact_in_ticks_across_tempo_changes() {
        let map = TempoMap::new(
            Tempo::new(120.0).unwrap(),
            &[TempoChange {
                tick: 960,
                bpm: Tempo::new(60.0).unwrap(),
            }],
            TimeSignature::default(),
            &[],
        )
        .unwrap();
        let l = lane(&[(0, -1.0, Curve::Linear), (1920, 1.0, Curve::Linear)]);
        let env = Envelope::compile(&lane_pieces(&l, 0.0).unwrap(), &map, 48_000);
        // Tick 960 (halfway) is at 0.5 s; tick 1440 at 1.0 s.
        assert!((env.value(24_000) - 0.5).abs() < 1e-6);
        assert!((env.value(48_000) - 0.75).abs() < 1e-6);
        assert!((env.value(72_000) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn steps_ramp_into_the_new_value_and_land_on_the_point() {
        let map = TempoMap::constant(Tempo::new(120.0).unwrap(), TimeSignature::default());
        let l = lane(&[(0, -1.0, Curve::Step), (960, 1.0, Curve::Linear)]);
        let env = Envelope::compile(&lane_pieces(&l, 0.0).unwrap(), &map, 48_000);
        assert_eq!(env.value(23_000), 0.0);
        assert_eq!(
            env.value(24_000),
            1.0,
            "the point's frame has the point's value"
        );
        let mut buf = vec![0.0f32; 200];
        env.fill(23_850, &mut buf);
        let max_step = buf
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.011, "de-clicked over 96 frames: {max_step}");
        // fill and value agree.
        for (k, v) in buf.iter().enumerate() {
            assert!((f64::from(*v) - env.value(23_850 + k as u64)).abs() < 1e-6);
        }
    }
}
