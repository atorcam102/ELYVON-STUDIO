//! Mix and parameter automation (schema 5).
//!
//! Every channel strip (track, auxiliary, master) owns a [`ChannelAutomation`]:
//! a mode and one lane per automated parameter. Audio regions may own lanes
//! too; inside a region's span its lane replaces the track lane, and the lane
//! moves with the region. Parameters are addressed by stable ids
//! ([`AutomationParam`]: the aux id of a send, the insert id of a processor),
//! never by position, so reordering inserts or sends keeps their automation.
//!
//! Values are stored in the parameter's own unit (dB, Hz, −1…1). Curves are
//! evaluated on a normalised scale (dB-linear for levels, logarithmic for
//! frequency, Q and release) over **musical time**: between two points the
//! fraction is measured in ticks, so a lane keeps its shape under tempo
//! changes. The engine compiles the same pieces to frames, splitting them at
//! tempo changes, and evaluates them per sample (see `elyvon-audio`).

use std::collections::HashSet;

use elyvon_core::Ticks;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edit::invalid;
use crate::error::{EditError, ValidationError};
use crate::mixer::{AuxId, ChannelRef, InsertId, Processor};
use crate::model::{Project, RegionId, TrackKind};

pub mod limits {
    /// Points per lane. Thousands of points per minute are normal for
    /// written automation; this keeps a lane under a few MiB.
    pub const MAX_POINTS: usize = 100_000;
    pub const MAX_LANES: usize = 64;
    /// Region lanes per region.
    pub const MAX_REGION_LANES: usize = 16;
    /// A batch of writes produced by one playback pass.
    pub const MAX_WRITES: usize = 4096;
}

/// How a channel's automation behaves during playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AutomationMode {
    /// Lanes are kept but ignored; the static strip values play.
    Off,
    /// Lanes play; moving a control never writes.
    #[default]
    Read,
    /// Lanes play; a touched control writes until it is released, then
    /// returns to the existing automation.
    Touch,
    /// Like Touch, but the last value is held until playback stops.
    Latch,
    /// Every automated parameter of the channel is overwritten from the
    /// start of playback with the current control values.
    Write,
}

impl AutomationMode {
    pub fn reads(self) -> bool {
        self != Self::Off
    }

    pub fn writes(self) -> bool {
        matches!(self, Self::Touch | Self::Latch | Self::Write)
    }
}

/// A parameter of a native insert processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum InsertParam {
    /// 0 = processing, 1 = bypassed (crossfaded, never switched).
    Bypass,
    TrimGain,
    FilterCutoff,
    FilterQ,
    LimiterCeiling,
    LimiterRelease,
}

impl InsertParam {
    /// Whether this parameter exists on `processor`.
    pub fn applies_to(self, processor: &Processor) -> bool {
        matches!(
            (self, processor),
            (Self::Bypass, _)
                | (Self::TrimGain, Processor::Trim { .. })
                | (Self::FilterCutoff | Self::FilterQ, Processor::Filter { .. })
                | (
                    Self::LimiterCeiling | Self::LimiterRelease,
                    Processor::Limiter { .. }
                )
        )
    }

    /// Current static value of this parameter on `processor`.
    pub fn value_of(self, bypass: bool, processor: &Processor) -> Option<f64> {
        Some(match (self, processor) {
            (Self::Bypass, _) => f64::from(u8::from(bypass)),
            (Self::TrimGain, Processor::Trim { gain_db }) => *gain_db,
            (Self::FilterCutoff, Processor::Filter { cutoff_hz, .. }) => *cutoff_hz,
            (Self::FilterQ, Processor::Filter { q, .. }) => *q,
            (Self::LimiterCeiling, Processor::Limiter { ceiling_db, .. }) => *ceiling_db,
            (Self::LimiterRelease, Processor::Limiter { release_ms, .. }) => *release_ms,
            _ => return None,
        })
    }
}

/// An automatable parameter of a channel strip, addressed by stable id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum AutomationParam {
    Volume,
    Pan,
    /// 0 = sounding, 1 = muted. Always stepped (with a short de-click ramp).
    Mute,
    /// Level of the send to `aux`.
    SendLevel {
        aux: AuxId,
    },
    Insert {
        id: InsertId,
        param: InsertParam,
    },
}

/// How values are interpolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// Linear in the stored unit (dB, pan).
    Linear,
    /// Linear in log2 of the stored unit (Hz, Q, ms).
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    pub min: f64,
    pub max: f64,
    pub scale: Scale,
    /// Two states only (0 or 1); curves are always steps.
    pub discrete: bool,
}

impl ParamSpec {
    const fn linear(min: f64, max: f64) -> Self {
        Self {
            min,
            max,
            scale: Scale::Linear,
            discrete: false,
        }
    }

    const fn log(min: f64, max: f64) -> Self {
        Self {
            min,
            max,
            scale: Scale::Log,
            discrete: false,
        }
    }

    const fn switch() -> Self {
        Self {
            min: 0.0,
            max: 1.0,
            scale: Scale::Linear,
            discrete: true,
        }
    }

    /// Clamps (and for switches rounds) a value into range.
    pub fn clamp(&self, v: f64) -> f64 {
        let v = if v.is_finite() { v } else { self.min };
        let v = v.clamp(self.min, self.max);
        if self.discrete { v.round() } else { v }
    }

    pub fn to_norm(&self, v: f64) -> f64 {
        let v = self.clamp(v);
        match self.scale {
            Scale::Linear => (v - self.min) / (self.max - self.min),
            Scale::Log => (v / self.min).log2() / (self.max / self.min).log2(),
        }
    }

    pub fn from_norm(&self, n: f64) -> f64 {
        let n = if n.is_finite() {
            n.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let v = match self.scale {
            Scale::Linear => self.min + n * (self.max - self.min),
            Scale::Log => self.min * (self.max / self.min).powf(n),
        };
        self.clamp(v)
    }
}

impl AutomationParam {
    pub fn spec(&self) -> ParamSpec {
        match self {
            Self::Volume | Self::SendLevel { .. } => ParamSpec::linear(-96.0, 6.0),
            Self::Pan => ParamSpec::linear(-1.0, 1.0),
            Self::Mute => ParamSpec::switch(),
            Self::Insert { param, .. } => match param {
                InsertParam::Bypass => ParamSpec::switch(),
                InsertParam::TrimGain => ParamSpec::linear(-48.0, 24.0),
                InsertParam::FilterCutoff => ParamSpec::log(20.0, 20_000.0),
                InsertParam::FilterQ => ParamSpec::log(0.3, 10.0),
                InsertParam::LimiterCeiling => ParamSpec::linear(-24.0, 0.0),
                InsertParam::LimiterRelease => ParamSpec::log(1.0, 1000.0),
            },
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Pan => "Pan",
            Self::Mute => "Mute",
            Self::SendLevel { .. } => "Send level",
            Self::Insert { param, .. } => match param {
                InsertParam::Bypass => "Insert bypass",
                InsertParam::TrimGain => "Trim gain",
                InsertParam::FilterCutoff => "Filter cutoff",
                InsertParam::FilterQ => "Filter Q",
                InsertParam::LimiterCeiling => "Limiter ceiling",
                InsertParam::LimiterRelease => "Limiter release",
            },
        }
    }
}

/// Shape of the segment from a point to the next one.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Curve {
    #[default]
    Linear,
    /// Holds the value until the next point (de-clicked by a 2 ms ramp).
    Step,
    /// One-dimensional cubic Bézier in value: control values `c1`, `c2`
    /// (0…1, relative to the segment's start and end values). `c1 = 1/3`,
    /// `c2 = 2/3` is linear; `0, 0` eases in; `1, 1` eases out.
    Bezier { c1: f64, c2: f64 },
}

impl Curve {
    /// Fraction of the way from the start to the end value at `x` (0…1).
    #[inline]
    pub fn shape(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match *self {
            Self::Linear => x,
            Self::Step => 0.0,
            Self::Bezier { c1, c2 } => bezier(c1, c2, x),
        }
    }

    fn is_valid(&self) -> bool {
        match *self {
            Self::Bezier { c1, c2 } => {
                c1.is_finite()
                    && c2.is_finite()
                    && (0.0..=1.0).contains(&c1)
                    && (0.0..=1.0).contains(&c2)
            }
            _ => true,
        }
    }
}

/// `B(x)` of a cubic Bézier from 0 to 1 with control values `c1`, `c2`.
#[inline]
pub fn bezier(c1: f64, c2: f64, x: f64) -> f64 {
    let u = 1.0 - x;
    3.0 * u * u * x * c1 + 3.0 * u * x * x * c2 + x * x * x
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutomationPoint {
    /// Unique within its lane; stable across edits.
    pub id: u32,
    /// Absolute for track lanes, relative to the region start for region lanes.
    #[ts(type = "number")]
    pub tick: Ticks,
    /// In the parameter's unit (dB, Hz, …).
    pub value: f64,
    /// Shape towards the next point.
    #[serde(default)]
    pub curve: Curve,
}

/// A point to add or write (ids are assigned by the core).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PointSpec {
    #[ts(type = "number")]
    pub tick: Ticks,
    pub value: f64,
    #[serde(default)]
    #[ts(optional)]
    pub curve: Option<Curve>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutomationLane {
    pub param: AutomationParam,
    /// Sorted by tick; several points may share a tick (a jump: the first is
    /// the value arriving there, the last the value leaving it).
    pub points: Vec<AutomationPoint>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ChannelAutomation {
    pub mode: AutomationMode,
    pub lanes: Vec<AutomationLane>,
}

impl ChannelAutomation {
    pub fn lane(&self, param: &AutomationParam) -> Option<&AutomationLane> {
        self.lanes.iter().find(|l| l.param == *param)
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.iter().all(|l| l.points.is_empty())
    }
}

/// Addresses one lane: a channel's, or (with `region`) a region's on that
/// channel's track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutomationTarget {
    pub channel: ChannelRef,
    #[serde(default)]
    #[ts(optional)]
    pub region: Option<RegionId>,
    pub param: AutomationParam,
}

/// One range replacement of a lane (written automation, paste-over).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutomationWrite {
    pub target: AutomationTarget,
    #[ts(type = "number")]
    pub from: Ticks,
    #[ts(type = "number")]
    pub to: Ticks,
    pub points: Vec<PointSpec>,
    /// Insert boundary points so the lane outside `[from, to]` keeps its
    /// previous values.
    pub keep_outside: bool,
}

// ---------------------------------------------------------------------------
// Evaluation (tick domain)

/// A piece of a lane between two positions, in normalised values. `t0`/`t1`
/// are the curve's own end ticks (a clipped piece keeps its original shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickPiece {
    pub from: f64,
    /// `f64::INFINITY` for the last piece.
    pub to: f64,
    pub t0: f64,
    pub t1: f64,
    pub v0: f64,
    pub v1: f64,
    pub curve: Curve,
}

impl TickPiece {
    pub fn constant(from: f64, to: f64, v: f64) -> Self {
        Self {
            from,
            to,
            t0: from,
            t1: from,
            v0: v,
            v1: v,
            curve: Curve::Step,
        }
    }

    /// Value (normalised) at tick `t`, which must lie in the curve's span.
    #[inline]
    pub fn value(&self, t: f64) -> f64 {
        if self.t1 <= self.t0 || self.v0 == self.v1 {
            return self.v0;
        }
        let x = (t - self.t0) / (self.t1 - self.t0);
        self.v0 + (self.v1 - self.v0) * self.curve.shape(x)
    }
}

/// The lane as pieces covering `[0, ∞)`, offset by `base` ticks (region
/// lanes). `None` for a lane without points.
pub fn lane_pieces(lane: &AutomationLane, base: f64) -> Option<Vec<TickPiece>> {
    let spec = lane.param.spec();
    let pts = &lane.points;
    let first = pts.first()?;
    let norm = |v: f64| spec.to_norm(v);
    let mut out = Vec::with_capacity(pts.len() + 1);
    let t_first = base + first.tick as f64;
    if t_first > 0.0 {
        out.push(TickPiece::constant(0.0, t_first, norm(first.value)));
    }
    for (i, p) in pts.iter().enumerate() {
        let t0 = base + p.tick as f64;
        let Some(next) = pts.get(i + 1) else {
            out.push(TickPiece::constant(t0, f64::INFINITY, norm(p.value)));
            break;
        };
        let t1 = base + next.tick as f64;
        if t1 <= t0 {
            continue;
        }
        let curve = if spec.discrete { Curve::Step } else { p.curve };
        let v0 = norm(p.value);
        let v1 = if curve == Curve::Step {
            v0
        } else {
            norm(next.value)
        };
        out.push(TickPiece {
            from: t0,
            to: t1,
            t0,
            t1,
            v0,
            v1,
            curve,
        });
    }
    Some(out)
}

/// Replaces `base` with `over` inside `[from, to)`. Both cover `[0, ∞)`.
pub fn overlay(base: &[TickPiece], over: &[TickPiece], from: f64, to: f64) -> Vec<TickPiece> {
    let mut out = Vec::with_capacity(base.len() + over.len() + 2);
    let clip = |p: &TickPiece, a: f64, b: f64| -> Option<TickPiece> {
        let from = p.from.max(a);
        let to = p.to.min(b);
        (to > from).then_some(TickPiece { from, to, ..*p })
    };
    out.extend(base.iter().filter_map(|p| clip(p, 0.0, from)));
    out.extend(over.iter().filter_map(|p| clip(p, from, to)));
    out.extend(base.iter().filter_map(|p| clip(p, to, f64::INFINITY)));
    out
}

/// Normalised value of `pieces` at tick `t` (the piece containing `t`).
pub fn pieces_value(pieces: &[TickPiece], t: f64) -> Option<f64> {
    let i = pieces.partition_point(|p| p.to <= t);
    pieces.get(i).map(|p| p.value(t.max(p.from)))
}

impl AutomationLane {
    /// Value (parameter unit) at tick `t` for an absolute (track) lane.
    pub fn value_at(&self, t: f64) -> Option<f64> {
        let pieces = lane_pieces(self, 0.0)?;
        pieces_value(&pieces, t).map(|n| self.param.spec().from_norm(n))
    }

    /// Value arriving at `t` from the left (before any jump at `t`).
    pub fn value_before(&self, t: f64) -> Option<f64> {
        let pieces = lane_pieces(self, 0.0)?;
        let i = pieces.partition_point(|p| p.to < t);
        let p = pieces.get(i)?;
        let at = t.clamp(p.from, if p.to.is_finite() { p.to } else { t });
        Some(self.param.spec().from_norm(p.value(at)))
    }

    fn next_id(&self) -> u32 {
        self.points.iter().map(|p| p.id).max().map_or(1, |m| m + 1)
    }

    fn sort(&mut self) {
        self.points.sort_by_key(|p| p.tick);
    }
}

// ---------------------------------------------------------------------------
// Access and validation

/// Static (unautomated) value of `param` on `channel`.
pub fn static_value(
    project: &Project,
    channel: ChannelRef,
    param: &AutomationParam,
) -> Option<f64> {
    let (volume, pan, muted, strip_sends, inserts) = match channel {
        ChannelRef::Track { id } => {
            let t = project.track(id)?;
            (
                t.volume_db,
                Some(t.pan),
                t.muted,
                Some(&t.strip.sends),
                &t.strip.inserts,
            )
        }
        ChannelRef::Aux { id } => {
            let a = project.mixer.aux(id)?;
            (
                a.volume_db,
                Some(a.pan),
                a.muted,
                Some(&a.strip.sends),
                &a.strip.inserts,
            )
        }
        ChannelRef::Master => {
            let m = &project.mixer.master;
            (m.volume_db, None, m.muted, None, &m.inserts)
        }
    };
    match param {
        AutomationParam::Volume => Some(volume),
        AutomationParam::Pan => pan,
        AutomationParam::Mute => Some(f64::from(u8::from(muted))),
        AutomationParam::SendLevel { aux } => strip_sends?
            .iter()
            .find(|s| s.target == *aux)
            .map(|s| s.level_db),
        AutomationParam::Insert { id, param } => inserts
            .iter()
            .find(|s| s.id == *id)
            .and_then(|s| param.value_of(s.bypass, &s.processor)),
    }
}

/// Whether `param` can be automated on `channel`.
pub fn param_exists(project: &Project, channel: ChannelRef, param: &AutomationParam) -> bool {
    if let ChannelRef::Track { id } = channel
        && project.track(id).is_some_and(|t| t.kind == TrackKind::Midi)
    {
        return false;
    }
    static_value(project, channel, param).is_some()
}

pub fn channel_automation(project: &Project, channel: ChannelRef) -> Option<&ChannelAutomation> {
    match channel {
        ChannelRef::Track { id } => project.track(id).map(|t| &t.automation),
        ChannelRef::Aux { id } => project.mixer.aux(id).map(|a| &a.automation),
        ChannelRef::Master => Some(&project.mixer.master.automation),
    }
}

fn channel_automation_mut(
    project: &mut Project,
    channel: ChannelRef,
) -> Result<&mut ChannelAutomation, EditError> {
    match channel {
        ChannelRef::Track { id } => project
            .track_mut(id)
            .map(|t| &mut t.automation)
            .ok_or(EditError::UnknownTrack(id)),
        ChannelRef::Aux { id } => project
            .mixer
            .aux_mut(id)
            .map(|a| &mut a.automation)
            .ok_or_else(|| invalid("unknown auxiliary channel")),
        ChannelRef::Master => Ok(&mut project.mixer.master.automation),
    }
}

/// The lane addressed by `target`, if it exists.
pub fn lane<'a>(project: &'a Project, target: &AutomationTarget) -> Option<&'a AutomationLane> {
    match target.region {
        None => channel_automation(project, target.channel)?.lane(&target.param),
        Some(rid) => {
            let (ti, ri) = project.find_region(rid)?;
            project.tracks[ti].regions[ri]
                .automation
                .iter()
                .find(|l| l.param == target.param)
        }
    }
}

fn check_target(project: &Project, target: &AutomationTarget) -> Result<(), EditError> {
    if !param_exists(project, target.channel, &target.param) {
        return Err(EditError::Incompatible(format!(
            "{} cannot be automated on this channel",
            target.param.label()
        )));
    }
    if let Some(rid) = target.region {
        let (ti, _) = project
            .find_region(rid)
            .ok_or(EditError::UnknownRegion(rid))?;
        if target.channel
            != (ChannelRef::Track {
                id: project.tracks[ti].id,
            })
        {
            return Err(invalid(
                "region automation belongs to the region's own track",
            ));
        }
    }
    Ok(())
}

/// The lane for `target`, created empty when missing.
fn lane_mut<'a>(
    project: &'a mut Project,
    target: &AutomationTarget,
    create: bool,
) -> Result<&'a mut AutomationLane, EditError> {
    check_target(project, target)?;
    let lanes = match target.region {
        None => &mut channel_automation_mut(project, target.channel)?.lanes,
        Some(rid) => {
            let (ti, ri) = project
                .find_region(rid)
                .ok_or(EditError::UnknownRegion(rid))?;
            &mut project.tracks[ti].regions[ri].automation
        }
    };
    if let Some(i) = lanes.iter().position(|l| l.param == target.param) {
        return Ok(&mut lanes[i]);
    }
    if !create {
        return Err(invalid("there is no automation for this parameter"));
    }
    let max = if target.region.is_some() {
        limits::MAX_REGION_LANES
    } else {
        limits::MAX_LANES
    };
    if lanes.len() >= max {
        return Err(EditError::Limit(format!("at most {max} automation lanes")));
    }
    lanes.push(AutomationLane {
        param: target.param,
        points: Vec::new(),
    });
    Ok(lanes.last_mut().expect("just pushed"))
}

pub(crate) fn validate_lanes(path: &str, lanes: &[AutomationLane]) -> Result<(), ValidationError> {
    let mut params = HashSet::with_capacity(lanes.len());
    for (i, lane) in lanes.iter().enumerate() {
        let lp = format!("{path}[{i}]");
        if !params.insert(lane.param) {
            return Err(ValidationError::new(lp, "duplicate automation lane"));
        }
        if lane.points.len() > limits::MAX_POINTS {
            return Err(ValidationError::new(
                format!("{lp}.points"),
                format!("more than {} points", limits::MAX_POINTS),
            ));
        }
        let spec = lane.param.spec();
        let mut ids = HashSet::with_capacity(lane.points.len());
        let mut prev = 0;
        for (j, p) in lane.points.iter().enumerate() {
            let pp = || format!("{lp}.points[{j}]");
            if !ids.insert(p.id) {
                return Err(ValidationError::new(pp(), "duplicate point id"));
            }
            if p.tick < prev {
                return Err(ValidationError::new(
                    pp(),
                    "points must be sorted by position",
                ));
            }
            prev = p.tick;
            if p.tick > crate::validate::limits::MAX_TICKS {
                return Err(ValidationError::new(pp(), "position out of range"));
            }
            if !p.value.is_finite() || p.value < spec.min || p.value > spec.max {
                return Err(ValidationError::new(
                    pp(),
                    format!(
                        "{} must be between {} and {}",
                        lane.param.label(),
                        spec.min,
                        spec.max
                    ),
                ));
            }
            if spec.discrete && p.value != 0.0 && p.value != 1.0 {
                return Err(ValidationError::new(pp(), "switch values are 0 or 1"));
            }
            if !p.curve.is_valid() {
                return Err(ValidationError::new(pp(), "Bézier controls must be 0…1"));
            }
        }
    }
    Ok(())
}

/// Checks every lane against its channel. Called by project validation.
pub(crate) fn validate_project(project: &Project) -> Result<(), ValidationError> {
    let check = |channel: ChannelRef, lanes: &[AutomationLane], path: &str| {
        validate_lanes(path, lanes)?;
        for (i, lane) in lanes.iter().enumerate() {
            if !param_exists(project, channel, &lane.param) {
                return Err(ValidationError::new(
                    format!("{path}[{i}]"),
                    format!("{} does not exist on this channel", lane.param.label()),
                ));
            }
        }
        Ok(())
    };
    for (ti, t) in project.tracks.iter().enumerate() {
        let channel = ChannelRef::Track { id: t.id };
        check(
            channel,
            &t.automation.lanes,
            &format!("tracks[{ti}].automation.lanes"),
        )?;
        for (ri, r) in t.regions.iter().enumerate() {
            if r.automation.is_empty() {
                continue;
            }
            let path = format!("tracks[{ti}].regions[{ri}].automation");
            if r.automation.len() > limits::MAX_REGION_LANES {
                return Err(ValidationError::new(path, "too many region lanes"));
            }
            check(channel, &r.automation, &path)?;
            if r.automation
                .iter()
                .any(|l| l.points.iter().any(|p| p.tick > r.length))
            {
                return Err(ValidationError::new(
                    path,
                    "points must lie inside the region",
                ));
            }
        }
    }
    for (i, a) in project.mixer.auxes.iter().enumerate() {
        check(
            ChannelRef::Aux { id: a.id },
            &a.automation.lanes,
            &format!("mixer.auxes[{i}].automation.lanes"),
        )?;
    }
    check(
        ChannelRef::Master,
        &project.mixer.master.automation.lanes,
        "mixer.master.automation.lanes",
    )
}

/// Drops lanes whose parameter no longer exists (a removed send or insert,
/// a processor changed to another type) and region lanes beyond the region.
/// Runs after every edit, so deleting an insert deletes its automation in
/// the same undo step.
pub(crate) fn prune(project: &mut Project) {
    let mut keep: Vec<(ChannelRef, AutomationParam)> = Vec::new();
    let channels: Vec<ChannelRef> = project
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
        .chain(std::iter::once(ChannelRef::Master))
        .collect();
    for &c in &channels {
        let Some(auto) = channel_automation(project, c) else {
            continue;
        };
        let mut params: Vec<AutomationParam> = auto.lanes.iter().map(|l| l.param).collect();
        if let ChannelRef::Track { id } = c
            && let Some(t) = project.track(id)
        {
            params.extend(
                t.regions
                    .iter()
                    .flat_map(|r| r.automation.iter().map(|l| l.param)),
            );
        }
        for p in params {
            if param_exists(project, c, &p) {
                keep.push((c, p));
            }
        }
    }
    let ok = |c: ChannelRef, p: &AutomationParam| keep.contains(&(c, *p));
    for t in &mut project.tracks {
        let c = ChannelRef::Track { id: t.id };
        t.automation.lanes.retain(|l| ok(c, &l.param));
        for r in &mut t.regions {
            r.automation.retain(|l| ok(c, &l.param));
            let len = r.length;
            for l in &mut r.automation {
                l.points.retain(|p| p.tick <= len);
            }
        }
    }
    for a in &mut project.mixer.auxes {
        let c = ChannelRef::Aux { id: a.id };
        a.automation.lanes.retain(|l| ok(c, &l.param));
    }
    project
        .mixer
        .master
        .automation
        .lanes
        .retain(|l| ok(ChannelRef::Master, &l.param));
}

// ---------------------------------------------------------------------------
// Edits

fn spec_point(spec: &ParamSpec, s: &PointSpec, id: u32) -> Result<AutomationPoint, EditError> {
    if s.tick > crate::validate::limits::MAX_TICKS {
        return Err(invalid("automation point out of range"));
    }
    if !s.value.is_finite() {
        return Err(invalid("automation values must be numbers"));
    }
    let curve = if spec.discrete {
        Curve::Step
    } else {
        s.curve.unwrap_or_default()
    };
    if !curve.is_valid() {
        return Err(invalid("Bézier controls must be 0…1"));
    }
    Ok(AutomationPoint {
        id,
        tick: s.tick,
        value: spec.clamp(s.value),
        curve,
    })
}

fn check_count(lane: &AutomationLane) -> Result<(), EditError> {
    if lane.points.len() > limits::MAX_POINTS {
        return Err(EditError::Limit(format!(
            "at most {} points per automation lane",
            limits::MAX_POINTS
        )));
    }
    Ok(())
}

pub(crate) fn set_mode(
    project: &mut Project,
    channel: ChannelRef,
    mode: AutomationMode,
) -> Result<(), EditError> {
    if let ChannelRef::Track { id } = channel
        && project.track(id).is_some_and(|t| t.kind == TrackKind::Midi)
    {
        return Err(EditError::Incompatible(
            "external MIDI tracks have no audio channel to automate".into(),
        ));
    }
    channel_automation_mut(project, channel)?.mode = mode;
    Ok(())
}

pub(crate) fn add_points(
    project: &mut Project,
    target: &AutomationTarget,
    points: &[PointSpec],
) -> Result<Vec<u32>, EditError> {
    if points.is_empty() {
        return Err(invalid("no points to add"));
    }
    let region_len = region_length(project, target)?;
    let lane = lane_mut(project, target, true)?;
    let spec = lane.param.spec();
    let mut created = Vec::with_capacity(points.len());
    for (id, s) in (lane.next_id()..).zip(points) {
        if region_len.is_some_and(|len| s.tick > len) {
            return Err(invalid("region automation must lie inside the region"));
        }
        lane.points.push(spec_point(&spec, s, id)?);
        created.push(id);
    }
    lane.sort();
    check_count(lane)?;
    Ok(created)
}

fn region_length(project: &Project, target: &AutomationTarget) -> Result<Option<Ticks>, EditError> {
    match target.region {
        None => Ok(None),
        Some(rid) => {
            let (ti, ri) = project
                .find_region(rid)
                .ok_or(EditError::UnknownRegion(rid))?;
            Ok(Some(project.tracks[ti].regions[ri].length))
        }
    }
}

fn check_ids(lane: &AutomationLane, ids: &[u32]) -> Result<(), EditError> {
    match ids
        .iter()
        .find(|id| !lane.points.iter().any(|p| p.id == **id))
    {
        Some(id) => Err(invalid(format!("automation point {id} does not exist"))),
        None => Ok(()),
    }
}

pub(crate) fn remove_points(
    project: &mut Project,
    target: &AutomationTarget,
    ids: &[u32],
) -> Result<(), EditError> {
    let lane = lane_mut(project, target, false)?;
    check_ids(lane, ids)?;
    lane.points.retain(|p| !ids.contains(&p.id));
    Ok(())
}

pub(crate) fn update_points(
    project: &mut Project,
    target: &AutomationTarget,
    points: &[AutomationPoint],
) -> Result<(), EditError> {
    let region_len = region_length(project, target)?;
    let lane = lane_mut(project, target, false)?;
    let spec = lane.param.spec();
    let ids: Vec<u32> = points.iter().map(|p| p.id).collect();
    check_ids(lane, &ids)?;
    for p in points {
        if region_len.is_some_and(|len| p.tick > len) {
            return Err(invalid("region automation must lie inside the region"));
        }
        let new = spec_point(
            &spec,
            &PointSpec {
                tick: p.tick,
                value: p.value,
                curve: Some(p.curve),
            },
            p.id,
        )?;
        if let Some(slot) = lane.points.iter_mut().find(|q| q.id == p.id) {
            *slot = new;
        }
    }
    lane.sort();
    Ok(())
}

/// Moves points by `delta_ticks` and `delta_norm` (fraction of the
/// parameter range). The earliest point clamps at zero; values clamp.
pub(crate) fn move_points(
    project: &mut Project,
    target: &AutomationTarget,
    ids: &[u32],
    delta_ticks: i64,
    delta_norm: f64,
    copy: bool,
) -> Result<Vec<u32>, EditError> {
    if !delta_norm.is_finite() {
        return Err(invalid("invalid value change"));
    }
    let region_len = region_length(project, target)?;
    let lane = lane_mut(project, target, false)?;
    check_ids(lane, ids)?;
    let spec = lane.param.spec();
    let earliest = lane
        .points
        .iter()
        .filter(|p| ids.contains(&p.id))
        .map(|p| p.tick)
        .min()
        .unwrap_or(0);
    let mut delta = delta_ticks.max(-(earliest.min(i64::MAX as u64) as i64));
    if let Some(len) = region_len {
        let latest = lane
            .points
            .iter()
            .filter(|p| ids.contains(&p.id))
            .map(|p| p.tick)
            .max()
            .unwrap_or(0);
        delta = delta.min(len.saturating_sub(latest) as i64);
    }
    let mut next = lane.next_id();
    let mut created = Vec::new();
    let mut moved = Vec::with_capacity(ids.len());
    for p in lane.points.iter_mut().filter(|p| ids.contains(&p.id)) {
        let mut q = *p;
        q.tick = q
            .tick
            .checked_add_signed(delta)
            .filter(|t| *t <= crate::validate::limits::MAX_TICKS)
            .ok_or_else(|| invalid("automation would move out of range"))?;
        q.value = spec.from_norm(spec.to_norm(q.value) + delta_norm);
        if copy {
            q.id = next;
            created.push(next);
            next += 1;
            moved.push(q);
        } else {
            *p = q;
        }
    }
    lane.points.extend(moved);
    lane.sort();
    check_count(lane)?;
    Ok(created)
}

pub(crate) fn remove_lane(
    project: &mut Project,
    target: &AutomationTarget,
) -> Result<(), EditError> {
    lane_mut(project, target, false)?;
    match target.region {
        None => channel_automation_mut(project, target.channel)?
            .lanes
            .retain(|l| l.param != target.param),
        Some(rid) => {
            let (ti, ri) = project
                .find_region(rid)
                .ok_or(EditError::UnknownRegion(rid))?;
            project.tracks[ti].regions[ri]
                .automation
                .retain(|l| l.param != target.param);
        }
    }
    Ok(())
}

/// Replaces the points in `[from, to]` (see [`AutomationWrite`]).
pub(crate) fn write(project: &mut Project, w: &AutomationWrite) -> Result<(), EditError> {
    if w.from > w.to {
        return Err(invalid("automation write range is reversed"));
    }
    if w.points.iter().any(|p| p.tick < w.from || p.tick > w.to) {
        return Err(invalid("written points must lie inside the written range"));
    }
    let fallback = static_value(project, w.target.channel, &w.target.param)
        .ok_or_else(|| invalid("unknown automation parameter"))?;
    let region_len = region_length(project, &w.target)?;
    if region_len.is_some_and(|len| w.to > len) {
        return Err(invalid("region automation must lie inside the region"));
    }
    let lane = lane_mut(project, &w.target, true)?;
    let spec = lane.param.spec();
    let had_points = !lane.points.is_empty();
    let before = if had_points {
        lane.value_before(w.from as f64).unwrap_or(fallback)
    } else {
        fallback
    };
    let after = if had_points {
        lane.value_at(w.to as f64).unwrap_or(fallback)
    } else {
        fallback
    };
    // Curve of the old segment that continues after `to`.
    let after_curve = lane
        .points
        .iter()
        .rev()
        .find(|p| p.tick <= w.to)
        .map_or(Curve::Linear, |p| p.curve);
    let mut kept_left: Vec<AutomationPoint> = lane
        .points
        .iter()
        .filter(|p| p.tick < w.from)
        .copied()
        .collect();
    let kept_right: Vec<AutomationPoint> = lane
        .points
        .iter()
        .filter(|p| p.tick > w.to)
        .copied()
        .collect();
    let mut id = lane.next_id();
    let mut mid = Vec::with_capacity(w.points.len() + 2);
    if w.keep_outside {
        mid.push(spec_point(
            &spec,
            &PointSpec {
                tick: w.from,
                value: before,
                curve: Some(Curve::Linear),
            },
            id,
        )?);
        id += 1;
    }
    for s in &w.points {
        mid.push(spec_point(&spec, s, id)?);
        id += 1;
    }
    // Stable by tick so the boundary stays first and last at equal ticks.
    mid.sort_by_key(|p| p.tick);
    if w.keep_outside {
        mid.push(spec_point(
            &spec,
            &PointSpec {
                tick: w.to,
                value: after,
                curve: Some(after_curve),
            },
            id,
        )?);
    }
    kept_left.extend(mid);
    kept_left.extend(kept_right);
    lane.points = kept_left;
    check_count(lane)?;
    Ok(())
}

/// Removes points that the lane does not need: within `tolerance` (fraction
/// of the range) of the line through their neighbours (Ramer–Douglas–Peucker
/// over linear segments). Steps, jumps and Bézier points are kept.
pub(crate) fn simplify(
    project: &mut Project,
    target: &AutomationTarget,
    tolerance: f64,
) -> Result<(), EditError> {
    if !(0.0..=0.5).contains(&tolerance) {
        return Err(invalid("tolerance must be 0…0.5"));
    }
    let lane = lane_mut(project, target, false)?;
    let spec = lane.param.spec();
    lane.points = thin(&lane.points, &spec, tolerance);
    Ok(())
}

/// See [`simplify`]. Keeps the first and last point.
pub fn thin(points: &[AutomationPoint], spec: &ParamSpec, tolerance: f64) -> Vec<AutomationPoint> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let n = points.len();
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    for i in 1..n - 1 {
        let (a, p, b) = (&points[i - 1], &points[i], &points[i + 1]);
        // Anchors: jumps, non-linear shapes and switches.
        if spec.discrete
            || p.curve != Curve::Linear
            || a.curve != Curve::Linear
            || a.tick == p.tick
            || p.tick == b.tick
        {
            keep[i] = true;
        }
    }
    let norm = |p: &AutomationPoint| spec.to_norm(p.value);
    let mut stack = Vec::new();
    let mut start = 0;
    for (end, _) in keep.iter().enumerate().skip(1).filter(|(_, k)| **k) {
        stack.push((start, end));
        start = end;
    }
    while let Some((s, e)) = stack.pop() {
        if e <= s + 1 {
            continue;
        }
        let (ts, te) = (points[s].tick as f64, points[e].tick as f64);
        let (vs, ve) = (norm(&points[s]), norm(&points[e]));
        let mut worst = (0.0, 0);
        for (i, p) in points.iter().enumerate().take(e).skip(s + 1) {
            let x = if te > ts {
                (p.tick as f64 - ts) / (te - ts)
            } else {
                0.0
            };
            let d = (norm(p) - (vs + (ve - vs) * x)).abs();
            if d > worst.0 {
                worst = (d, i);
            }
        }
        if worst.0 > tolerance {
            keep[worst.1] = true;
            stack.push((s, worst.1));
            stack.push((worst.1, e));
        }
    }
    let mut out: Vec<AutomationPoint> = points
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(p, _)| *p)
        .collect();
    if spec.discrete {
        // Repeated equal states carry no information.
        out.dedup_by(|b, a| a.value == b.value && a.tick != b.tick);
    }
    out
}

/// Approximate heap bytes of a project's automation (history budgeting).
pub fn approx_bytes(project: &Project) -> usize {
    let point = std::mem::size_of::<AutomationPoint>();
    let lanes = |l: &[AutomationLane]| l.iter().map(|x| x.points.len() * point + 64).sum::<usize>();
    project
        .tracks
        .iter()
        .map(|t| {
            lanes(&t.automation.lanes)
                + t.regions
                    .iter()
                    .map(|r| lanes(&r.automation))
                    .sum::<usize>()
        })
        .sum::<usize>()
        + project
            .mixer
            .auxes
            .iter()
            .map(|a| lanes(&a.automation.lanes))
            .sum::<usize>()
        + lanes(&project.mixer.master.automation.lanes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(points: &[(u64, f64, Curve)]) -> AutomationLane {
        AutomationLane {
            param: AutomationParam::Volume,
            points: points
                .iter()
                .enumerate()
                .map(|(i, &(tick, value, curve))| AutomationPoint {
                    id: i as u32 + 1,
                    tick,
                    value,
                    curve,
                })
                .collect(),
        }
    }

    #[test]
    fn specs_round_trip_and_log_scales_are_geometric() {
        let cutoff = AutomationParam::Insert {
            id: InsertId::new(),
            param: InsertParam::FilterCutoff,
        }
        .spec();
        assert!((cutoff.from_norm(0.5) - (20.0f64 * 20_000.0).sqrt()).abs() < 1e-6);
        for v in [20.0, 440.0, 20_000.0] {
            assert!((cutoff.from_norm(cutoff.to_norm(v)) - v).abs() < 1e-6);
        }
        let mute = AutomationParam::Mute.spec();
        assert_eq!(mute.clamp(0.4), 0.0);
        assert_eq!(mute.clamp(0.6), 1.0);
    }

    #[test]
    fn bezier_controls_shape_the_segment() {
        assert!(
            (Curve::Bezier {
                c1: 1.0 / 3.0,
                c2: 2.0 / 3.0
            }
            .shape(0.37)
                - 0.37)
                .abs()
                < 1e-12
        );
        let ease_in = Curve::Bezier { c1: 0.0, c2: 0.0 };
        assert!(ease_in.shape(0.5) < 0.5);
        assert_eq!(ease_in.shape(0.0), 0.0);
        assert_eq!(ease_in.shape(1.0), 1.0);
        assert_eq!(Curve::Step.shape(0.99), 0.0);
    }

    #[test]
    fn values_follow_points_steps_and_jumps() {
        let l = lane(&[
            (0, -12.0, Curve::Linear),
            (960, 0.0, Curve::Step),
            (1920, -24.0, Curve::Linear),
            (1920, -6.0, Curve::Linear),
            (2880, -6.0, Curve::Linear),
        ]);
        assert!((l.value_at(480.0).unwrap() - (-6.0)).abs() < 1e-9);
        assert_eq!(l.value_at(1500.0).unwrap(), 0.0, "step holds");
        assert_eq!(
            l.value_before(1920.0).unwrap(),
            0.0,
            "arrives at the held value"
        );
        assert_eq!(
            l.value_at(1920.0).unwrap(),
            -6.0,
            "leaves with the last point of the jump"
        );
        assert_eq!(l.value_at(10_000.0).unwrap(), -6.0);
    }

    #[test]
    fn overlay_replaces_a_span_and_keeps_shapes() {
        let base = lane_pieces(
            &lane(&[(0, -96.0, Curve::Linear), (3840, 6.0, Curve::Linear)]),
            0.0,
        )
        .unwrap();
        let over = vec![TickPiece::constant(0.0, f64::INFINITY, 0.25)];
        let merged = overlay(&base, &over, 960.0, 1920.0);
        assert!((pieces_value(&merged, 480.0).unwrap() - 0.125).abs() < 1e-12);
        assert_eq!(pieces_value(&merged, 1000.0).unwrap(), 0.25);
        assert!((pieces_value(&merged, 2880.0).unwrap() - 0.75).abs() < 1e-12);
    }

    #[test]
    fn thinning_keeps_shape_within_tolerance() {
        let points: Vec<AutomationPoint> = (0..1000u32)
            .map(|i| AutomationPoint {
                id: i,
                tick: u64::from(i) * 10,
                value: -48.0 + 48.0 * (f64::from(i) / 999.0),
                curve: Curve::Linear,
            })
            .collect();
        let spec = AutomationParam::Volume.spec();
        let thinned = thin(&points, &spec, 0.001);
        assert_eq!(thinned.len(), 2, "a straight ramp needs two points");
        let wobble: Vec<AutomationPoint> = points
            .iter()
            .map(|p| AutomationPoint {
                value: (p.value + 6.0 * (p.tick as f64 / 500.0).sin()).clamp(-96.0, 6.0),
                ..*p
            })
            .collect();
        let thinned = thin(&wobble, &spec, 0.002);
        assert!(
            thinned.len() < 200 && thinned.len() > 10,
            "{}",
            thinned.len()
        );
        let l = AutomationLane {
            param: AutomationParam::Volume,
            points: thinned,
        };
        for p in &wobble {
            let err = spec.to_norm(l.value_at(p.tick as f64).unwrap()) - spec.to_norm(p.value);
            assert!(err.abs() <= 0.0021, "error {err} at {}", p.tick);
        }
    }
}
