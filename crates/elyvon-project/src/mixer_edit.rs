//! Mixer edit commands (Phase 09): channel strips, sends, inserts, auxiliary
//! channels, groups and folders. Routing is validated by
//! [`crate::mixer::check_routing`] after every command, so an edit that would
//! create a cycle fails and leaves the project unchanged.

use elyvon_core::Color;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edit::{EditOutcome, invalid};
use crate::error::EditError;
use crate::mixer::{
    AuxChannel, AuxId, ChannelFormat, ChannelRef, FolderId, GroupId, InsertId, InsertSlot,
    MixGroup, OutputTarget, Processor, Send, StripRouting, TrackFolder, limits,
};
use crate::model::{Project, TRACK_PALETTE, Track, TrackId};
use crate::validate::validate_name;

/// Optional strip changes; absent fields stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct StripPatch {
    #[ts(optional)]
    pub name: Option<String>,
    #[ts(optional)]
    pub color: Option<Color>,
    #[ts(optional)]
    pub volume_db: Option<f64>,
    #[ts(optional)]
    pub pan: Option<f64>,
    #[ts(optional)]
    pub muted: Option<bool>,
    #[ts(optional)]
    pub soloed: Option<bool>,
    #[ts(optional)]
    pub solo_safe: Option<bool>,
    #[ts(optional)]
    pub output: Option<OutputTarget>,
    #[ts(optional)]
    pub format: Option<ChannelFormat>,
}

/// Which strip parameters a group edit links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Linked {
    Volume,
    Pan,
    Mute,
    Solo,
}

fn level_ok(v: f64) -> bool {
    v.is_finite() && (Track::MIN_VOLUME_DB..=Track::MAX_VOLUME_DB).contains(&v)
}

fn pan_ok(v: f64) -> bool {
    v.is_finite() && (-1.0..=1.0).contains(&v)
}

struct StripMut<'a> {
    volume_db: Option<&'a mut f64>,
    pan: Option<&'a mut f64>,
    muted: Option<&'a mut bool>,
    soloed: Option<&'a mut bool>,
    strip: Option<&'a mut StripRouting>,
}

fn strip_mut(project: &mut Project, channel: ChannelRef) -> Result<StripMut<'_>, EditError> {
    match channel {
        ChannelRef::Track { id } => {
            let t = project.track_mut(id).ok_or(EditError::UnknownTrack(id))?;
            Ok(StripMut {
                volume_db: Some(&mut t.volume_db),
                pan: Some(&mut t.pan),
                muted: Some(&mut t.muted),
                soloed: Some(&mut t.soloed),
                strip: Some(&mut t.strip),
            })
        }
        ChannelRef::Aux { id } => {
            let a = project
                .mixer
                .aux_mut(id)
                .ok_or_else(|| invalid("unknown auxiliary channel"))?;
            Ok(StripMut {
                volume_db: Some(&mut a.volume_db),
                pan: Some(&mut a.pan),
                muted: Some(&mut a.muted),
                soloed: Some(&mut a.soloed),
                strip: Some(&mut a.strip),
            })
        }
        ChannelRef::Master => {
            let m = &mut project.mixer.master;
            Ok(StripMut {
                volume_db: Some(&mut m.volume_db),
                pan: None,
                muted: Some(&mut m.muted),
                soloed: None,
                strip: None,
            })
        }
    }
}

fn group_of(project: &Project, channel: ChannelRef) -> Option<GroupId> {
    match channel {
        ChannelRef::Track { id } => project.track(id).and_then(|t| t.strip.group),
        ChannelRef::Aux { id } => project.mixer.aux(id).and_then(|a| a.strip.group),
        ChannelRef::Master => None,
    }
}

/// `channel` plus the other members of its group when `what` is linked.
pub(crate) fn linked_channels(
    project: &Project,
    channel: ChannelRef,
    what: Linked,
) -> Vec<ChannelRef> {
    let mut out = vec![channel];
    let Some(gid) = group_of(project, channel) else {
        return out;
    };
    let Some(group) = project.mixer.group(gid) else {
        return out;
    };
    let linked = match what {
        Linked::Volume => group.link_volume,
        Linked::Pan => group.link_pan,
        Linked::Mute => group.link_mute,
        Linked::Solo => group.link_solo,
    };
    if !linked {
        return out;
    }
    for t in &project.tracks {
        let c = ChannelRef::Track { id: t.id };
        if t.strip.group == Some(gid) && c != channel {
            out.push(c);
        }
    }
    for a in &project.mixer.auxes {
        let c = ChannelRef::Aux { id: a.id };
        if a.strip.group == Some(gid) && c != channel {
            out.push(c);
        }
    }
    out
}

/// Sets `channel`'s volume; linked group members move by the same dB
/// (clamped to the fader range).
pub(crate) fn set_volume(
    project: &mut Project,
    channel: ChannelRef,
    db: f64,
) -> Result<(), EditError> {
    if !level_ok(db) {
        return Err(invalid("volume out of range"));
    }
    let current = *strip_mut(project, channel)?
        .volume_db
        .ok_or_else(|| invalid("this channel has no fader"))?;
    let delta = db - current;
    for c in linked_channels(project, channel, Linked::Volume) {
        if let Some(v) = strip_mut(project, c)?.volume_db {
            *v = if c == channel {
                db
            } else {
                (*v + delta).clamp(Track::MIN_VOLUME_DB, Track::MAX_VOLUME_DB)
            };
        }
    }
    Ok(())
}

pub(crate) fn set_pan(
    project: &mut Project,
    channel: ChannelRef,
    pan: f64,
) -> Result<(), EditError> {
    if !pan_ok(pan) {
        return Err(invalid("pan out of range"));
    }
    let current = *strip_mut(project, channel)?
        .pan
        .ok_or_else(|| invalid("this channel has no pan control"))?;
    let delta = pan - current;
    for c in linked_channels(project, channel, Linked::Pan) {
        if let Some(v) = strip_mut(project, c)?.pan {
            *v = if c == channel {
                pan
            } else {
                (*v + delta).clamp(-1.0, 1.0)
            };
        }
    }
    Ok(())
}

pub(crate) fn set_flag(
    project: &mut Project,
    channels: &[ChannelRef],
    what: Linked,
    on: bool,
) -> Result<(), EditError> {
    let mut all = Vec::new();
    for &c in channels {
        for linked in linked_channels(project, c, what) {
            if !all.contains(&linked) {
                all.push(linked);
            }
        }
    }
    for c in all {
        let s = strip_mut(project, c)?;
        let slot = match what {
            Linked::Mute => s.muted,
            Linked::Solo => s.soloed,
            _ => None,
        };
        match slot {
            Some(v) => *v = on,
            None => return Err(invalid("this channel cannot be soloed")),
        }
    }
    Ok(())
}

pub(crate) fn set_strip(
    project: &mut Project,
    channel: ChannelRef,
    patch: &StripPatch,
) -> Result<(), EditError> {
    if let Some(name) = &patch.name {
        validate_name("name", name)?;
        match channel {
            ChannelRef::Track { id } => {
                project
                    .track_mut(id)
                    .ok_or(EditError::UnknownTrack(id))?
                    .name = name.trim().into()
            }
            ChannelRef::Aux { id } => {
                project
                    .mixer
                    .aux_mut(id)
                    .ok_or_else(|| invalid("unknown auxiliary channel"))?
                    .name = name.trim().into()
            }
            ChannelRef::Master => return Err(invalid("the master channel cannot be renamed")),
        }
    }
    if let Some(color) = patch.color {
        match channel {
            ChannelRef::Track { id } => {
                project
                    .track_mut(id)
                    .ok_or(EditError::UnknownTrack(id))?
                    .color = color
            }
            ChannelRef::Aux { id } => {
                project
                    .mixer
                    .aux_mut(id)
                    .ok_or_else(|| invalid("unknown auxiliary channel"))?
                    .color = color
            }
            ChannelRef::Master => return Err(invalid("the master channel has no colour")),
        }
    }
    if let Some(db) = patch.volume_db {
        set_volume(project, channel, db)?;
    }
    if let Some(pan) = patch.pan {
        set_pan(project, channel, pan)?;
    }
    if let Some(m) = patch.muted {
        set_flag(project, &[channel], Linked::Mute, m)?;
    }
    if let Some(s) = patch.soloed {
        set_flag(project, &[channel], Linked::Solo, s)?;
    }
    if patch.solo_safe.is_some() || patch.output.is_some() || patch.format.is_some() {
        let strip = strip_mut(project, channel)?
            .strip
            .ok_or_else(|| invalid("the master channel has no routing"))?;
        if let Some(v) = patch.solo_safe {
            strip.solo_safe = v;
        }
        if let Some(v) = patch.output {
            strip.output = v;
        }
        if let Some(v) = patch.format {
            strip.format = v;
        }
    }
    Ok(())
}

fn routing_mut(project: &mut Project, channel: ChannelRef) -> Result<&mut StripRouting, EditError> {
    strip_mut(project, channel)?
        .strip
        .ok_or_else(|| invalid("the master channel has no sends"))
}

fn inserts_mut(
    project: &mut Project,
    channel: ChannelRef,
) -> Result<&mut Vec<InsertSlot>, EditError> {
    match channel {
        ChannelRef::Master => Ok(&mut project.mixer.master.inserts),
        other => Ok(&mut routing_mut(project, other)?.inserts),
    }
}

/// Adds, changes (`Some`) or removes (`None`) the send to `target`.
pub(crate) fn set_send(
    project: &mut Project,
    channel: ChannelRef,
    target: AuxId,
    send: Option<Send>,
) -> Result<(), EditError> {
    if project.mixer.aux(target).is_none() {
        return Err(invalid("unknown auxiliary channel"));
    }
    let strip = routing_mut(project, channel)?;
    match send {
        Some(mut s) => {
            s.target = target;
            match strip.sends.iter_mut().find(|x| x.target == target) {
                Some(existing) => *existing = s,
                None => {
                    if strip.sends.len() >= limits::MAX_SENDS {
                        return Err(EditError::Limit(format!(
                            "at most {} sends",
                            limits::MAX_SENDS
                        )));
                    }
                    strip.sends.push(s);
                }
            }
        }
        None => {
            let before = strip.sends.len();
            strip.sends.retain(|x| x.target != target);
            if strip.sends.len() == before {
                return Err(invalid("there is no send to that channel"));
            }
        }
    }
    Ok(())
}

pub(crate) fn add_insert(
    project: &mut Project,
    channel: ChannelRef,
    processor: Processor,
    index: Option<usize>,
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    processor.validate().map_err(|e| invalid(e.to_string()))?;
    let inserts = inserts_mut(project, channel)?;
    if inserts.len() >= limits::MAX_INSERTS {
        return Err(EditError::Limit(format!(
            "at most {} inserts",
            limits::MAX_INSERTS
        )));
    }
    let slot = InsertSlot {
        id: InsertId::new(),
        bypass: false,
        processor,
    };
    outcome.created_inserts.push(slot.id);
    let at = index.unwrap_or(inserts.len()).min(inserts.len());
    inserts.insert(at, slot);
    Ok(())
}

pub(crate) fn remove_insert(
    project: &mut Project,
    channel: ChannelRef,
    id: InsertId,
) -> Result<(), EditError> {
    let inserts = inserts_mut(project, channel)?;
    let before = inserts.len();
    inserts.retain(|s| s.id != id);
    if inserts.len() == before {
        return Err(invalid("unknown insert"));
    }
    Ok(())
}

pub(crate) fn move_insert(
    project: &mut Project,
    channel: ChannelRef,
    id: InsertId,
    to: usize,
) -> Result<(), EditError> {
    let inserts = inserts_mut(project, channel)?;
    let from = inserts
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| invalid("unknown insert"))?;
    let slot = inserts.remove(from);
    let to = to.min(inserts.len());
    inserts.insert(to, slot);
    Ok(())
}

pub(crate) fn set_insert(
    project: &mut Project,
    channel: ChannelRef,
    id: InsertId,
    bypass: bool,
    processor: Processor,
) -> Result<(), EditError> {
    processor.validate().map_err(|e| invalid(e.to_string()))?;
    let slot = inserts_mut(project, channel)?
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| invalid("unknown insert"))?;
    if std::mem::discriminant(&slot.processor) != std::mem::discriminant(&processor) {
        return Err(invalid(
            "an insert keeps its processor type; remove it and add another",
        ));
    }
    slot.bypass = bypass;
    slot.processor = processor;
    Ok(())
}

fn unique_name(existing: impl Iterator<Item = String>, base: &str) -> String {
    let names: Vec<String> = existing.collect();
    (1..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !names.contains(n))
        .unwrap_or_else(|| base.into())
}

pub(crate) fn add_aux(
    project: &mut Project,
    name: Option<&str>,
    outcome: &mut EditOutcome,
) -> Result<AuxId, EditError> {
    if project.mixer.auxes.len() >= limits::MAX_AUXES {
        return Err(EditError::Limit(format!(
            "at most {} auxiliary channels",
            limits::MAX_AUXES
        )));
    }
    let name = match name {
        Some(n) => {
            validate_name("name", n)?;
            n.trim().to_owned()
        }
        None => unique_name(project.mixer.auxes.iter().map(|a| a.name.clone()), "Aux"),
    };
    let color = TRACK_PALETTE[(project.mixer.auxes.len() + 3) % TRACK_PALETTE.len()];
    let aux = AuxChannel::new(name, color);
    let id = aux.id;
    outcome.created_auxes.push(id);
    project.mixer.auxes.push(aux);
    Ok(id)
}

/// Removes auxiliary channels. Strips that used them as output go to the
/// master; sends to them are removed. Nothing is left dangling.
pub(crate) fn remove_auxes(project: &mut Project, ids: &[AuxId]) -> Result<(), EditError> {
    for id in ids {
        if project.mixer.aux(*id).is_none() {
            return Err(invalid("unknown auxiliary channel"));
        }
    }
    project.mixer.auxes.retain(|a| !ids.contains(&a.id));
    let fix = |s: &mut StripRouting| {
        if matches!(s.output, OutputTarget::Aux { id } if ids.contains(&id)) {
            s.output = OutputTarget::Master;
        }
        s.sends.retain(|x| !ids.contains(&x.target));
    };
    project.tracks.iter_mut().for_each(|t| fix(&mut t.strip));
    project
        .mixer
        .auxes
        .iter_mut()
        .for_each(|a| fix(&mut a.strip));
    Ok(())
}

/// Creates a group with linked volume, mute and solo and adds `members`.
pub(crate) fn add_group(
    project: &mut Project,
    name: Option<&str>,
    members: &[ChannelRef],
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    if project.mixer.groups.len() >= limits::MAX_GROUPS {
        return Err(EditError::Limit(format!(
            "at most {} groups",
            limits::MAX_GROUPS
        )));
    }
    let name = match name {
        Some(n) => {
            validate_name("name", n)?;
            n.trim().to_owned()
        }
        None => unique_name(project.mixer.groups.iter().map(|g| g.name.clone()), "Group"),
    };
    let group = MixGroup {
        id: GroupId::new(),
        name,
        color: TRACK_PALETTE[project.mixer.groups.len() % TRACK_PALETTE.len()],
        link_volume: true,
        link_pan: false,
        link_mute: true,
        link_solo: true,
        link_edit: false,
    };
    let id = group.id;
    outcome.created_groups.push(id);
    project.mixer.groups.push(group);
    set_group(project, members, Some(id))
}

pub(crate) fn update_group(project: &mut Project, group: &MixGroup) -> Result<(), EditError> {
    validate_name("name", &group.name)?;
    let g = project
        .mixer
        .groups
        .iter_mut()
        .find(|g| g.id == group.id)
        .ok_or_else(|| invalid("unknown group"))?;
    *g = group.clone();
    Ok(())
}

pub(crate) fn remove_group(project: &mut Project, id: GroupId) -> Result<(), EditError> {
    if project.mixer.group(id).is_none() {
        return Err(invalid("unknown group"));
    }
    project.mixer.groups.retain(|g| g.id != id);
    for t in &mut project.tracks {
        if t.strip.group == Some(id) {
            t.strip.group = None;
        }
    }
    for a in &mut project.mixer.auxes {
        if a.strip.group == Some(id) {
            a.strip.group = None;
        }
    }
    Ok(())
}

pub(crate) fn set_group(
    project: &mut Project,
    channels: &[ChannelRef],
    group: Option<GroupId>,
) -> Result<(), EditError> {
    if group.is_some_and(|g| project.mixer.group(g).is_none()) {
        return Err(invalid("unknown group"));
    }
    for &c in channels {
        routing_mut(project, c)?.group = group;
    }
    Ok(())
}

pub(crate) fn add_folder(
    project: &mut Project,
    name: Option<&str>,
    tracks: &[TrackId],
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    if project.mixer.folders.len() >= limits::MAX_FOLDERS {
        return Err(EditError::Limit(format!(
            "at most {} folders",
            limits::MAX_FOLDERS
        )));
    }
    let name = match name {
        Some(n) => {
            validate_name("name", n)?;
            n.trim().to_owned()
        }
        None => unique_name(
            project.mixer.folders.iter().map(|f| f.name.clone()),
            "Folder",
        ),
    };
    let folder = TrackFolder {
        id: FolderId::new(),
        name,
        color: TRACK_PALETTE[(project.mixer.folders.len() + 5) % TRACK_PALETTE.len()],
        collapsed: false,
    };
    let id = folder.id;
    outcome.created_folders.push(id);
    project.mixer.folders.push(folder);
    set_track_folder(project, tracks, Some(id))
}

pub(crate) fn update_folder(project: &mut Project, folder: &TrackFolder) -> Result<(), EditError> {
    validate_name("name", &folder.name)?;
    let f = project
        .mixer
        .folders
        .iter_mut()
        .find(|f| f.id == folder.id)
        .ok_or_else(|| invalid("unknown folder"))?;
    *f = folder.clone();
    Ok(())
}

pub(crate) fn remove_folder(project: &mut Project, id: FolderId) -> Result<(), EditError> {
    if !project.mixer.folders.iter().any(|f| f.id == id) {
        return Err(invalid("unknown folder"));
    }
    project.mixer.folders.retain(|f| f.id != id);
    for t in &mut project.tracks {
        if t.folder == Some(id) {
            t.folder = None;
        }
    }
    Ok(())
}

pub(crate) fn set_track_folder(
    project: &mut Project,
    ids: &[TrackId],
    folder: Option<FolderId>,
) -> Result<(), EditError> {
    if folder.is_some_and(|f| !project.mixer.folders.iter().any(|x| x.id == f)) {
        return Err(invalid("unknown folder"));
    }
    for id in ids {
        project
            .track_mut(*id)
            .ok_or(EditError::UnknownTrack(*id))?
            .folder = folder;
    }
    Ok(())
}
