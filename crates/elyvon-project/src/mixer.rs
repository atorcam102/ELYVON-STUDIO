//! Mixer and routing description (schema 4).
//!
//! Every track and auxiliary channel is a channel strip. A strip sends its
//! post-fader signal to exactly one [`OutputTarget`] and may feed auxiliary
//! channels through [`Send`]s. The resulting routing graph must be acyclic:
//! [`check_routing`] runs on every validation, so a cycle is rejected before
//! an edit is applied and the previous project is restored. There is no
//! feedback policy that allows cycles; this is deliberate (no delayed feedback
//! paths, no runaway gain).
//!
//! Insert slots hold typed native processors. Third-party plug-in formats are
//! not hosted yet and have no variant here.

use std::collections::{HashMap, HashSet};

use elyvon_core::Color;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::ValidationError;
use crate::model::{Project, TrackId};

macro_rules! mixer_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        #[ts(export, type = "string")]
        pub struct $name(pub uuid::Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

mixer_id!(
    /// An auxiliary channel (bus return).
    AuxId
);
mixer_id!(GroupId);
mixer_id!(InsertId);
mixer_id!(FolderId);

pub mod limits {
    pub const MAX_AUXES: usize = 64;
    pub const MAX_SENDS: usize = 8;
    pub const MAX_INSERTS: usize = 8;
    pub const MAX_GROUPS: usize = 32;
    pub const MAX_FOLDERS: usize = 64;
    pub const MIN_SEND_DB: f64 = -96.0;
    pub const MAX_SEND_DB: f64 = 6.0;
    pub const MAX_LOOKAHEAD_MS: f64 = 20.0;
}

/// Where a strip's post-fader signal goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum OutputTarget {
    Master,
    Aux { id: AuxId },
}

/// Mono strips sum their input to one channel and pan it with the project
/// pan law; stereo strips use balance. `Auto` is mono only for audio tracks
/// whose regions all play mono files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChannelFormat {
    #[default]
    Auto,
    Mono,
    Stereo,
}

/// Gain of a mono source at the centre of the pan range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PanLaw {
    /// Constant power, −3 dB per side at centre (the 0.2.x behaviour).
    #[default]
    ConstantPower,
    /// Constant power compensated to 0 dB at centre (+3 dB at the extremes).
    ConstantPowerCompensated,
    /// Linear, −6 dB per side at centre.
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Send {
    pub target: AuxId,
    pub level_db: f64,
    /// Taps before the fader and pan instead of after them.
    pub pre_fader: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FilterMode {
    HighPass,
    LowPass,
}

/// A native processor. Parameters are validated; processing is in
/// `elyvon-audio::dsp`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Processor {
    /// Gain trim, −48 … +24 dB.
    Trim { gain_db: f64 },
    /// Polarity, channel swap and mono summing.
    Utility {
        invert_left: bool,
        invert_right: bool,
        swap: bool,
        mono: bool,
    },
    /// 12 dB/oct biquad, 20 Hz … 20 kHz, Q 0.3 … 10.
    Filter {
        mode: FilterMode,
        cutoff_hz: f64,
        q: f64,
    },
    /// Look-ahead peak limiter. Its look-ahead is reported as latency and
    /// compensated on parallel paths.
    Limiter {
        ceiling_db: f64,
        lookahead_ms: f64,
        release_ms: f64,
    },
}

impl Processor {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Trim { .. } => "Trim",
            Self::Utility { .. } => "Utility",
            Self::Filter { .. } => "Filter",
            Self::Limiter { .. } => "Limiter",
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let finite = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        match *self {
            Self::Trim { gain_db } if !finite(gain_db, -48.0, 24.0) => {
                Err("trim gain must be −48 … +24 dB")
            }
            Self::Filter { cutoff_hz, q, .. }
                if !finite(cutoff_hz, 20.0, 20_000.0) || !finite(q, 0.3, 10.0) =>
            {
                Err("filter cutoff must be 20 Hz … 20 kHz and Q 0.3 … 10")
            }
            Self::Limiter {
                ceiling_db,
                lookahead_ms,
                release_ms,
            } if !finite(ceiling_db, -24.0, 0.0)
                || !finite(lookahead_ms, 0.0, limits::MAX_LOOKAHEAD_MS)
                || !finite(release_ms, 1.0, 1000.0) =>
            {
                Err("limiter ceiling must be −24 … 0 dB, look-ahead 0 … 20 ms, release 1 … 1000 ms")
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InsertSlot {
    pub id: InsertId,
    pub bypass: bool,
    pub processor: Processor,
}

/// Routing and processing shared by tracks and auxiliary channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct StripRouting {
    pub output: OutputTarget,
    pub sends: Vec<Send>,
    pub inserts: Vec<InsertSlot>,
    /// Stays audible when other channels are soloed.
    pub solo_safe: bool,
    pub group: Option<GroupId>,
    pub format: ChannelFormat,
}

impl Default for StripRouting {
    fn default() -> Self {
        Self {
            output: OutputTarget::Master,
            sends: Vec::new(),
            inserts: Vec::new(),
            solo_safe: false,
            group: None,
            format: ChannelFormat::Auto,
        }
    }
}

/// An auxiliary channel: receives sends and track outputs, has its own strip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AuxChannel {
    pub id: AuxId,
    pub name: String,
    pub color: Color,
    pub volume_db: f64,
    pub pan: f64,
    pub muted: bool,
    pub soloed: bool,
    pub strip: StripRouting,
    #[serde(default)]
    pub automation: crate::automation::ChannelAutomation,
}

impl AuxChannel {
    pub fn new(name: impl Into<String>, color: Color) -> Self {
        Self {
            id: AuxId::new(),
            name: name.into(),
            color,
            volume_db: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            strip: StripRouting {
                format: ChannelFormat::Stereo,
                ..StripRouting::default()
            },
            automation: crate::automation::ChannelAutomation::default(),
        }
    }
}

/// Linked parameters of the channels that belong to a group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MixGroup {
    pub id: GroupId,
    pub name: String,
    pub color: Color,
    /// Volume changes move every member by the same number of dB.
    pub link_volume: bool,
    /// Pan changes move every member by the same amount.
    pub link_pan: bool,
    pub link_mute: bool,
    pub link_solo: bool,
    /// Region edits (split, trim, move, slip, delete, mute) on one member apply
    /// to the regions of the other members with the same position and length,
    /// keeping multitrack recordings phase-locked (schema 5).
    #[serde(default)]
    pub link_edit: bool,
}

/// A named set of tracks shown together (and collapsible) in the mixer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrackFolder {
    pub id: FolderId,
    pub name: String,
    pub color: Color,
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct MasterChannel {
    pub volume_db: f64,
    pub muted: bool,
    pub inserts: Vec<InsertSlot>,
    pub automation: crate::automation::ChannelAutomation,
}

impl Default for MasterChannel {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            muted: false,
            inserts: Vec::new(),
            automation: crate::automation::ChannelAutomation::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Mixer {
    pub auxes: Vec<AuxChannel>,
    pub groups: Vec<MixGroup>,
    pub folders: Vec<TrackFolder>,
    pub master: MasterChannel,
    pub pan_law: PanLaw,
}

impl Mixer {
    pub fn aux(&self, id: AuxId) -> Option<&AuxChannel> {
        self.auxes.iter().find(|a| a.id == id)
    }

    pub fn aux_mut(&mut self, id: AuxId) -> Option<&mut AuxChannel> {
        self.auxes.iter_mut().find(|a| a.id == id)
    }

    pub fn group(&self, id: GroupId) -> Option<&MixGroup> {
        self.groups.iter().find(|g| g.id == id)
    }
}

/// A channel strip in the routing graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ChannelRef {
    Track { id: TrackId },
    Aux { id: AuxId },
    Master,
}

/// Edges of the routing graph: `(from, to)` for every output and send.
pub fn routing_edges(project: &Project) -> Vec<(ChannelRef, ChannelRef)> {
    let target = |t: OutputTarget| match t {
        OutputTarget::Master => ChannelRef::Master,
        OutputTarget::Aux { id } => ChannelRef::Aux { id },
    };
    let mut edges = Vec::new();
    let mut strip = |from: ChannelRef, s: &StripRouting| {
        edges.push((from, target(s.output)));
        for send in &s.sends {
            edges.push((from, ChannelRef::Aux { id: send.target }));
        }
    };
    for t in &project.tracks {
        strip(ChannelRef::Track { id: t.id }, &t.strip);
    }
    for a in &project.mixer.auxes {
        strip(ChannelRef::Aux { id: a.id }, &a.strip);
    }
    edges
}

fn aux_name(project: &Project, id: AuxId) -> String {
    project
        .mixer
        .aux(id)
        .map_or_else(|| "missing aux".into(), |a| a.name.clone())
}

/// Rejects references to missing channels and any routing cycle. Returns the
/// auxiliary channels in a processing order (sources before destinations).
pub fn check_routing(project: &Project) -> Result<Vec<AuxId>, ValidationError> {
    let auxes: HashSet<AuxId> = project.mixer.auxes.iter().map(|a| a.id).collect();
    let mut next: HashMap<AuxId, Vec<AuxId>> = HashMap::new();
    let mut indegree: HashMap<AuxId, usize> = auxes.iter().map(|id| (*id, 0)).collect();
    for (from, to) in routing_edges(project) {
        let ChannelRef::Aux { id: to } = to else {
            continue;
        };
        if !auxes.contains(&to) {
            return Err(ValidationError::new(
                "mixer",
                "a channel is routed to an auxiliary channel that does not exist",
            ));
        }
        match from {
            ChannelRef::Aux { id: from } if from == to => {
                return Err(ValidationError::new(
                    "mixer",
                    format!("routing cycle: {} feeds itself", aux_name(project, from)),
                ));
            }
            ChannelRef::Aux { id: from } => {
                next.entry(from).or_default().push(to);
                *indegree.entry(to).or_default() += 1;
            }
            _ => {}
        }
    }
    // Kahn's algorithm in mixer order, so equal graphs give equal orders.
    let mut order = Vec::with_capacity(auxes.len());
    let mut ready: Vec<AuxId> = project
        .mixer
        .auxes
        .iter()
        .filter(|a| indegree[&a.id] == 0)
        .map(|a| a.id)
        .collect();
    ready.reverse();
    while let Some(id) = ready.pop() {
        order.push(id);
        for to in next.get(&id).into_iter().flatten() {
            let n = indegree.get_mut(to).expect("known aux");
            *n -= 1;
            if *n == 0 {
                ready.push(*to);
            }
        }
    }
    if order.len() != auxes.len() {
        let stuck: Vec<String> = project
            .mixer
            .auxes
            .iter()
            .filter(|a| !order.contains(&a.id))
            .map(|a| a.name.clone())
            .collect();
        return Err(ValidationError::new(
            "mixer",
            format!(
                "routing cycle between {}; feedback paths are not allowed",
                stuck.join(", ")
            ),
        ));
    }
    Ok(order)
}

/// Whether routing `from` to aux `to` (as output or send) would be accepted.
pub fn can_route(project: &Project, from: ChannelRef, to: AuxId) -> bool {
    let mut trial = project.clone();
    let mut strip = match from {
        ChannelRef::Track { id } => match trial.track_mut(id) {
            Some(t) => std::mem::take(&mut t.strip),
            None => return false,
        },
        ChannelRef::Aux { id } => match trial.mixer.aux_mut(id) {
            Some(a) => std::mem::take(&mut a.strip),
            None => return false,
        },
        ChannelRef::Master => return false,
    };
    strip.sends.push(Send {
        target: to,
        level_db: 0.0,
        pre_fader: false,
        muted: false,
    });
    match from {
        ChannelRef::Track { id } => {
            if let Some(t) = trial.track_mut(id) {
                t.strip = strip;
            }
        }
        ChannelRef::Aux { id } => {
            if let Some(a) = trial.mixer.aux_mut(id) {
                a.strip = strip;
            }
        }
        ChannelRef::Master => {}
    }
    check_routing(&trial).is_ok()
}

pub(crate) fn validate_strip(path: &str, strip: &StripRouting) -> Result<(), ValidationError> {
    if strip.sends.len() > limits::MAX_SENDS {
        return Err(ValidationError::new(
            format!("{path}.sends"),
            format!("at most {} sends", limits::MAX_SENDS),
        ));
    }
    let mut targets = HashSet::new();
    for (i, s) in strip.sends.iter().enumerate() {
        if !s.level_db.is_finite()
            || !(limits::MIN_SEND_DB..=limits::MAX_SEND_DB).contains(&s.level_db)
        {
            return Err(ValidationError::new(
                format!("{path}.sends[{i}].levelDb"),
                "must be between -96 and 6 dB",
            ));
        }
        if !targets.insert(s.target) {
            return Err(ValidationError::new(
                format!("{path}.sends[{i}].target"),
                "only one send per destination",
            ));
        }
    }
    validate_inserts(&format!("{path}.inserts"), &strip.inserts)
}

pub(crate) fn validate_inserts(path: &str, inserts: &[InsertSlot]) -> Result<(), ValidationError> {
    if inserts.len() > limits::MAX_INSERTS {
        return Err(ValidationError::new(
            path,
            format!("at most {} inserts", limits::MAX_INSERTS),
        ));
    }
    let mut ids = HashSet::new();
    for (i, slot) in inserts.iter().enumerate() {
        if !ids.insert(slot.id) {
            return Err(ValidationError::new(
                format!("{path}[{i}].id"),
                "duplicate insert id",
            ));
        }
        slot.processor
            .validate()
            .map_err(|e| ValidationError::new(format!("{path}[{i}].processor"), e))?;
    }
    Ok(())
}

pub(crate) fn validate_mixer(project: &Project) -> Result<(), ValidationError> {
    use crate::validate::validate_name;
    let m = &project.mixer;
    if m.auxes.len() > limits::MAX_AUXES
        || m.groups.len() > limits::MAX_GROUPS
        || m.folders.len() > limits::MAX_FOLDERS
    {
        return Err(ValidationError::new(
            "mixer",
            "too many auxiliary channels, groups or folders",
        ));
    }
    let level = |v: f64| v.is_finite() && (-96.0..=6.0).contains(&v);
    if !level(m.master.volume_db) {
        return Err(ValidationError::new(
            "mixer.master.volumeDb",
            "must be between -96 and 6 dB",
        ));
    }
    validate_inserts("mixer.master.inserts", &m.master.inserts)?;
    let mut ids = HashSet::new();
    for (i, a) in m.auxes.iter().enumerate() {
        let p = format!("mixer.auxes[{i}]");
        if !ids.insert(a.id) {
            return Err(ValidationError::new(format!("{p}.id"), "duplicate aux id"));
        }
        validate_name(&format!("{p}.name"), &a.name)?;
        if !level(a.volume_db) || !a.pan.is_finite() || !(-1.0..=1.0).contains(&a.pan) {
            return Err(ValidationError::new(p, "volume or pan out of range"));
        }
        validate_strip(&format!("{p}.strip"), &a.strip)?;
    }
    let mut group_ids = HashSet::new();
    for (i, g) in m.groups.iter().enumerate() {
        if !group_ids.insert(g.id) {
            return Err(ValidationError::new(
                format!("mixer.groups[{i}].id"),
                "duplicate group id",
            ));
        }
        validate_name(&format!("mixer.groups[{i}].name"), &g.name)?;
    }
    let mut folder_ids = HashSet::new();
    for (i, f) in m.folders.iter().enumerate() {
        if !folder_ids.insert(f.id) {
            return Err(ValidationError::new(
                format!("mixer.folders[{i}].id"),
                "duplicate folder id",
            ));
        }
        validate_name(&format!("mixer.folders[{i}].name"), &f.name)?;
    }
    let strips = project
        .tracks
        .iter()
        .map(|t| (&t.strip, t.folder))
        .chain(m.auxes.iter().map(|a| (&a.strip, None)));
    for (strip, folder) in strips {
        if strip.group.is_some_and(|g| !group_ids.contains(&g)) {
            return Err(ValidationError::new(
                "mixer",
                "a channel refers to a missing group",
            ));
        }
        if folder.is_some_and(|f| !folder_ids.contains(&f)) {
            return Err(ValidationError::new(
                "mixer",
                "a track refers to a missing folder",
            ));
        }
    }
    check_routing(project).map(|_| ())
}
