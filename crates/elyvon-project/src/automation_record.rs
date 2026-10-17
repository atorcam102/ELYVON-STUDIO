//! Capture of automation written by moving controls during playback.
//!
//! The desktop shell feeds an [`AutomationRecorder`] with the engine position
//! (~30 Hz) and with control gestures (begin/move/end, stamped with the
//! engine position). When playback stops, [`AutomationRecorder::finish`]
//! turns everything into one [`EditCommand::WriteAutomation`] — a single undo
//! step. Nothing is written for channels in Off/Read mode, while stopped, or
//! into region lanes.
//!
//! A *pass* is a stretch of continuous playback. A cycle wrap or a locate
//! starts a new pass; later passes overwrite earlier ones where they overlap
//! (the writes are applied in order).

use std::collections::HashMap;

use elyvon_core::Ticks;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::{
    AutomationMode, AutomationParam, AutomationPoint, AutomationTarget, AutomationWrite, Curve,
    PointSpec, channel_automation, static_value, thin,
};
use crate::edit::EditCommand;
use crate::mixer::ChannelRef;
use crate::model::Project;

/// Phase of a control gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TouchPhase {
    Begin,
    Move,
    End,
}

/// Seconds over which a released Touch control returns to the lane.
pub const TOUCH_RELEASE_SECONDS: f64 = 0.2;
/// Thinning tolerance for written data, as a fraction of the range.
pub const WRITE_TOLERANCE: f64 = 0.002;
/// A control that does not move holds its value; between two samples
/// further apart than twice this, the value is held and then ramps over this
/// long into the next sample.
pub const HOLD_RAMP_SECONDS: f64 = 0.03;
/// A forward jump larger than this between observations is a locate.
const FORWARD_JUMP_TICKS: Ticks = 2 * 3840;
/// Positions read on different threads may arrive slightly out of order; a
/// smaller step back is not a locate.
const JITTER_TICKS: Ticks = 480;

#[derive(Debug, Clone)]
struct Touch {
    target: AutomationTarget,
    mode: AutomationMode,
    pass: usize,
    samples: Vec<(Ticks, f64)>,
    /// Touch mode: where the control was released.
    released: Option<Ticks>,
}

#[derive(Debug, Clone)]
pub struct AutomationRecorder {
    passes: Vec<(Ticks, Ticks)>,
    pass_start: Ticks,
    last: Ticks,
    /// Engine cycle-wrap counter at the last observation.
    epoch: u64,
    touches: Vec<Touch>,
    /// Touches still writing (held, or latched until stop).
    active: HashMap<AutomationTarget, usize>,
    /// Write-mode channels: every lane they had at the start, with the
    /// static value written where the control is not moved.
    write_targets: Vec<(AutomationTarget, f64)>,
}

impl AutomationRecorder {
    /// Starts recording at `tick` for the channels of `project`. `epoch` is
    /// the engine's cycle-wrap counter.
    pub fn start(project: &Project, tick: Ticks, epoch: u64) -> Self {
        let mut write_targets = Vec::new();
        let channels = project
            .tracks
            .iter()
            .map(|t| ChannelRef::Track { id: t.id })
            .chain(
                project
                    .mixer
                    .auxes
                    .iter()
                    .map(|a| ChannelRef::Aux { id: a.id }),
            )
            .chain(std::iter::once(ChannelRef::Master));
        for channel in channels {
            let Some(auto) = channel_automation(project, channel) else {
                continue;
            };
            if auto.mode != AutomationMode::Write {
                continue;
            }
            for lane in auto.lanes.iter().filter(|l| !l.points.is_empty()) {
                if let Some(v) = static_value(project, channel, &lane.param) {
                    write_targets.push((
                        AutomationTarget {
                            channel,
                            region: None,
                            param: lane.param,
                        },
                        v,
                    ));
                }
            }
        }
        Self {
            passes: Vec::new(),
            pass_start: tick,
            last: tick,
            epoch,
            touches: Vec::new(),
            active: HashMap::new(),
            write_targets,
        }
    }

    /// Whether anything would be written so far.
    pub fn is_empty(&self) -> bool {
        self.touches.is_empty() && self.write_targets.is_empty()
    }

    /// Engine position while playing. Detects cycle wraps (`epoch` changes)
    /// and locates (position jumps).
    pub fn observe(&mut self, tick: Ticks, epoch: u64) {
        if epoch != self.epoch
            || tick.saturating_add(JITTER_TICKS) < self.last
            || tick > self.last.saturating_add(FORWARD_JUMP_TICKS)
        {
            self.epoch = epoch;
            self.new_pass(tick);
        } else {
            self.last = self.last.max(tick);
        }
    }

    fn new_pass(&mut self, tick: Ticks) {
        let end = self.last;
        self.passes.push((self.pass_start, end));
        let pass = self.passes.len();
        // Held and latched controls continue in the new pass.
        let held: Vec<(AutomationTarget, usize)> = self.active.drain().collect();
        for (target, i) in held {
            let value = self.touches[i].samples.last().map_or(0.0, |s| s.1);
            if let Some(last) = self.touches[i].samples.last_mut()
                && last.0 < end
            {
                self.touches[i].samples.push((end, value));
            }
            let mode = self.touches[i].mode;
            self.touches.push(Touch {
                target,
                mode,
                pass,
                samples: vec![(tick, value)],
                released: None,
            });
            self.active.insert(target, self.touches.len() - 1);
        }
        self.pass_start = tick;
        self.last = tick;
    }

    /// A control gesture during playback. Returns whether it is recorded
    /// (only for channels in Touch, Latch or Write mode).
    pub fn touch(
        &mut self,
        target: AutomationTarget,
        mode: AutomationMode,
        phase: TouchPhase,
        value: f64,
        tick: Ticks,
        epoch: u64,
    ) -> bool {
        if !mode.writes() || target.region.is_some() || !value.is_finite() {
            return false;
        }
        self.observe(tick, epoch);
        let at = self.last;
        let pass = self.passes.len();
        match self.active.get(&target).copied() {
            Some(i) => {
                let t = &mut self.touches[i];
                t.samples.push((at, value));
                if phase == TouchPhase::End && mode == AutomationMode::Touch {
                    t.released = Some(at);
                    self.active.remove(&target);
                }
            }
            None => {
                let released =
                    (phase == TouchPhase::End && mode == AutomationMode::Touch).then_some(at);
                self.touches.push(Touch {
                    target,
                    mode,
                    pass,
                    samples: vec![(at, value)],
                    released,
                });
                if released.is_none() {
                    self.active.insert(target, self.touches.len() - 1);
                }
            }
        }
        true
    }

    /// Ends recording at `tick` and returns the edit to apply, if any.
    pub fn finish(mut self, tick: Ticks, epoch: u64, project: &Project) -> Option<EditCommand> {
        self.observe(tick, epoch);
        self.passes.push((self.pass_start, self.last));
        let map = project.transport.tempo_map().ok();
        let release_end = |t: Ticks| -> Ticks {
            map.as_ref().map_or(t + 192, |m| {
                let secs = m.seconds_at(t as f64) + TOUCH_RELEASE_SECONDS;
                (m.tick_at_seconds(secs).round() as Ticks).max(t + 1)
            })
        };
        let mut writes = Vec::new();
        // Touch and Latch channels: one write per gesture.
        for t in self
            .touches
            .iter()
            .filter(|t| t.mode != AutomationMode::Write)
        {
            let (_, pass_end) = self.passes[t.pass];
            let Some(&(begin, _)) = t.samples.first() else {
                continue;
            };
            let mut samples = t.samples.clone();
            let to = match t.released {
                Some(r) => release_end(r),
                None => {
                    // Latched (or held at stop): keep the last value to the end.
                    let v = samples.last().map_or(0.0, |s| s.1);
                    if samples.last().is_some_and(|s| s.0 < pass_end) {
                        samples.push((pass_end, v));
                    }
                    pass_end
                }
            };
            writes.push(make_write(
                t.target,
                begin,
                to.max(begin),
                &samples,
                map.as_ref(),
            ));
        }
        // Write channels: every lane (and every touched parameter) over every pass.
        let mut targets: Vec<(AutomationTarget, f64)> = self.write_targets.clone();
        for t in self
            .touches
            .iter()
            .filter(|t| t.mode == AutomationMode::Write)
        {
            if !targets.iter().any(|(x, _)| *x == t.target)
                && let Some(v) = static_value(project, t.target.channel, &t.target.param)
            {
                targets.push((t.target, v));
            }
        }
        for (pass, &(start, end)) in self.passes.iter().enumerate() {
            if end <= start {
                continue;
            }
            for &(target, base) in &targets {
                let mut samples = vec![(start, base)];
                for t in self.touches.iter().filter(|t| {
                    t.mode == AutomationMode::Write && t.pass == pass && t.target == target
                }) {
                    samples.extend(t.samples.iter().copied());
                }
                samples.sort_by_key(|s| s.0);
                let v = samples.last().map_or(base, |s| s.1);
                samples.push((end, v));
                writes.push(make_write(target, start, end, &samples, map.as_ref()));
            }
        }
        if writes.is_empty() {
            return None;
        }
        writes.truncate(crate::automation::limits::MAX_WRITES);
        Some(EditCommand::WriteAutomation { writes })
    }
}

fn make_write(
    target: AutomationTarget,
    from: Ticks,
    to: Ticks,
    samples: &[(Ticks, f64)],
    map: Option<&elyvon_core::TempoMap>,
) -> AutomationWrite {
    let spec = target.param.spec();
    let curve = if spec.discrete {
        Curve::Step
    } else {
        Curve::Linear
    };
    let hold = |t: Ticks| -> Ticks {
        map.map_or(58, |m| {
            let s = m.seconds_at(t as f64) - HOLD_RAMP_SECONDS;
            t.saturating_sub(m.tick_at_seconds(s.max(0.0)).round() as Ticks)
                .max(1)
        })
    };
    let mut points: Vec<AutomationPoint> = Vec::with_capacity(samples.len() * 2);
    let mut prev: Option<(Ticks, f64)> = None;
    for &(tick, value) in samples.iter().filter(|(t, _)| *t >= from && *t <= to) {
        let value = spec.clamp(value);
        if let Some((pt, pv)) = prev
            && !spec.discrete
            && pv != value
        {
            let h = hold(tick);
            if tick > pt + 2 * h {
                points.push(AutomationPoint {
                    id: points.len() as u32,
                    tick: tick - h,
                    value: pv,
                    curve,
                });
            }
        }
        points.push(AutomationPoint {
            id: points.len() as u32,
            tick,
            value,
            curve,
        });
        prev = Some((tick, value));
    }
    let points = thin(&points, &spec, WRITE_TOLERANCE);
    AutomationWrite {
        target,
        from,
        to,
        points: points
            .iter()
            .map(|p| PointSpec {
                tick: p.tick,
                value: p.value,
                curve: Some(p.curve),
            })
            .collect(),
        keep_outside: true,
    }
}

/// Parameters a channel exposes for writing (for UI menus and tests).
pub fn writable_params(project: &Project, channel: ChannelRef) -> Vec<AutomationParam> {
    let mut out = Vec::new();
    for p in [
        AutomationParam::Volume,
        AutomationParam::Pan,
        AutomationParam::Mute,
    ] {
        if crate::automation::param_exists(project, channel, &p) {
            out.push(p);
        }
    }
    let (sends, inserts) = match channel {
        ChannelRef::Track { id } => match project.track(id) {
            Some(t) => (t.strip.sends.as_slice(), t.strip.inserts.as_slice()),
            None => return out,
        },
        ChannelRef::Aux { id } => match project.mixer.aux(id) {
            Some(a) => (a.strip.sends.as_slice(), a.strip.inserts.as_slice()),
            None => return out,
        },
        ChannelRef::Master => (&[][..], project.mixer.master.inserts.as_slice()),
    };
    for s in sends {
        out.push(AutomationParam::SendLevel { aux: s.target });
    }
    use crate::automation::InsertParam as I;
    for slot in inserts {
        for param in [
            I::Bypass,
            I::TrimGain,
            I::FilterCutoff,
            I::FilterQ,
            I::LimiterCeiling,
            I::LimiterRelease,
        ] {
            if param.applies_to(&slot.processor) {
                out.push(AutomationParam::Insert { id: slot.id, param });
            }
        }
    }
    out.retain(|p| crate::automation::param_exists(project, channel, p));
    out
}
