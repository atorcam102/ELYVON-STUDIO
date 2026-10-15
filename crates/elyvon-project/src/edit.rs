//! Edit commands: the only way the UI changes a project.
//!
//! Each command is applied atomically by [`apply`]: on error the caller's
//! project is restored, so a half-applied edit can never be observed.
//! Continuous gestures (dragging a fader or a region) are previewed in the UI
//! and committed as a single command on release, giving one undo step each.

use elyvon_core::{Color, Key, MeterChange, Tempo, TempoChange, Ticks, TimeSignature};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::EditError;
use crate::model::{
    MarkerId, MediaAsset, MediaId, MediaInfo, MetronomeSettings, Project, Region, RegionContent,
    RegionId, Track, TrackId, TrackKind,
};
use crate::timeline_edit as tl;
use crate::validate::{limits, validate, validate_name};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum EditCommand {
    RenameProject {
        name: String,
    },
    SetTempo {
        bpm: f64,
    },
    SetTimeSignature {
        numerator: u8,
        denominator: u8,
    },
    SetKey {
        key: Key,
    },
    SetLoop {
        enabled: bool,
        #[ts(type = "number")]
        start: Ticks,
        #[ts(type = "number")]
        end: Ticks,
    },
    AddTrack {
        kind: TrackKind,
        #[ts(optional)]
        name: Option<String>,
        /// Insert position; appended when absent.
        #[ts(optional, type = "number")]
        index: Option<usize>,
    },
    RemoveTracks {
        ids: Vec<TrackId>,
    },
    DuplicateTracks {
        ids: Vec<TrackId>,
    },
    RenameTrack {
        id: TrackId,
        name: String,
    },
    SetTrackColor {
        id: TrackId,
        color: Color,
    },
    SetTracksMuted {
        ids: Vec<TrackId>,
        muted: bool,
    },
    SetTracksSoloed {
        ids: Vec<TrackId>,
        soloed: bool,
    },
    SetTrackVolume {
        id: TrackId,
        volume_db: f64,
    },
    SetTrackPan {
        id: TrackId,
        pan: f64,
    },
    MoveTrack {
        id: TrackId,
        #[ts(type = "number")]
        to_index: usize,
    },
    /// Creates an empty MIDI region on an instrument or MIDI track.
    CreateMidiRegion {
        track_id: TrackId,
        #[ts(type = "number")]
        start: Ticks,
        #[ts(type = "number")]
        length: Ticks,
    },
    /// Moves regions in time and, optionally, by whole tracks up or down.
    MoveRegions {
        ids: Vec<RegionId>,
        #[ts(type = "number")]
        delta_ticks: i64,
        track_offset: i32,
    },
    RemoveRegions {
        ids: Vec<RegionId>,
    },
    RenameRegion {
        id: RegionId,
        name: String,
    },
    SetSampleRate {
        sample_rate: u32,
    },
    /// Replaces the tempo steps after the start (sorted by tick).
    SetTempoChanges {
        changes: Vec<TempoChange>,
    },
    /// Replaces the metre changes after bar 1 (sorted by bar).
    SetMeterChanges {
        changes: Vec<MeterChange>,
    },
    SetMetronome {
        settings: MetronomeSettings,
    },
    SetCountIn {
        bars: u8,
    },
    SetPreRoll {
        bars: u8,
    },
    SetPunch {
        enabled: bool,
        #[ts(type = "number")]
        start: Ticks,
        #[ts(type = "number")]
        end: Ticks,
    },
    AddMarker {
        #[ts(type = "number")]
        tick: Ticks,
        #[ts(optional)]
        name: Option<String>,
    },
    UpdateMarker {
        id: MarkerId,
        name: String,
        #[ts(type = "number")]
        tick: Ticks,
    },
    RemoveMarkers {
        ids: Vec<MarkerId>,
    },
    /// Adds decoded files to the media pool and places regions for them.
    /// Built by the native importer, which measured each file.
    ImportAudio {
        assets: Vec<MediaAsset>,
        placements: Vec<AudioPlacement>,
    },
    /// Points a media-pool entry at another file (e.g. a moved, missing file).
    RelinkMedia {
        id: MediaId,
        path: String,
        info: Option<MediaInfo>,
    },
    /// Splits each region that spans `at` into two (non-destructive for audio).
    SplitRegions {
        ids: Vec<RegionId>,
        #[ts(type = "number")]
        at: Ticks,
    },
    /// Moves a region's edges. Audio keeps its timeline position: trimming the
    /// start advances the source offset instead of moving the sound.
    TrimRegion {
        id: RegionId,
        #[ts(type = "number")]
        start: Ticks,
        #[ts(type = "number")]
        end: Ticks,
    },
    /// Copies regions right after the selection on their own tracks.
    DuplicateRegions {
        ids: Vec<RegionId>,
    },
    /// Inserts copies (fresh ids) of clipboard regions at `at`, starting on
    /// `track_id` and keeping relative track offsets.
    PasteRegions {
        track_id: TrackId,
        #[ts(type = "number")]
        at: Ticks,
        items: Vec<ClipboardRegion>,
    },
    SetRegionsMuted {
        ids: Vec<RegionId>,
        muted: bool,
    },
    /// Gain, fades (source frames) and looping of an audio region.
    SetAudioRegion {
        id: RegionId,
        gain_db: f64,
        #[ts(type = "number")]
        fade_in: u64,
        #[ts(type = "number")]
        fade_out: u64,
        looped: bool,
    },
}

/// Where an imported file goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PlacementTrack {
    Existing {
        id: TrackId,
    },
    /// A new audio track named after the file, appended at the end.
    New,
    /// The track used by the previous placement of the same command.
    SameAsPrevious,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioPlacement {
    pub media_id: MediaId,
    pub track: PlacementTrack,
    #[ts(type = "number")]
    pub start: Ticks,
}

/// A region captured by Copy/Cut, with its track relative to the first one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ClipboardRegion {
    pub track_offset: u32,
    pub region: Region,
}

impl EditCommand {
    /// Short human-readable description for undo/redo menus.
    pub fn label(&self) -> &'static str {
        match self {
            Self::RenameProject { .. } => "Rename Project",
            Self::SetTempo { .. } => "Change Tempo",
            Self::SetTimeSignature { .. } => "Change Time Signature",
            Self::SetKey { .. } => "Change Key",
            Self::SetLoop { .. } => "Change Loop",
            Self::AddTrack { .. } => "New Track",
            Self::RemoveTracks { .. } => "Delete Tracks",
            Self::DuplicateTracks { .. } => "Duplicate Tracks",
            Self::RenameTrack { .. } => "Rename Track",
            Self::SetTrackColor { .. } => "Change Track Colour",
            Self::SetTracksMuted { .. } => "Mute",
            Self::SetTracksSoloed { .. } => "Solo",
            Self::SetTrackVolume { .. } => "Change Volume",
            Self::SetTrackPan { .. } => "Change Pan",
            Self::MoveTrack { .. } => "Move Track",
            Self::CreateMidiRegion { .. } => "New MIDI Region",
            Self::MoveRegions { .. } => "Move Regions",
            Self::RemoveRegions { .. } => "Delete Regions",
            Self::RenameRegion { .. } => "Rename Region",
            Self::SetSampleRate { .. } => "Change Sample Rate",
            Self::SetTempoChanges { .. } => "Change Tempo Map",
            Self::SetMeterChanges { .. } => "Change Time Signature Map",
            Self::SetMetronome { .. } => "Metronome Settings",
            Self::SetCountIn { .. } => "Count-in",
            Self::SetPreRoll { .. } => "Pre-roll",
            Self::SetPunch { .. } => "Change Punch Range",
            Self::AddMarker { .. } => "Add Marker",
            Self::UpdateMarker { .. } => "Edit Marker",
            Self::RemoveMarkers { .. } => "Delete Markers",
            Self::ImportAudio { .. } => "Import Audio",
            Self::RelinkMedia { .. } => "Relink Audio File",
            Self::SplitRegions { .. } => "Split Regions",
            Self::TrimRegion { .. } => "Trim Region",
            Self::DuplicateRegions { .. } => "Duplicate Regions",
            Self::PasteRegions { .. } => "Paste Regions",
            Self::SetRegionsMuted { .. } => "Mute Regions",
            Self::SetAudioRegion { .. } => "Change Region Gain/Fades",
        }
    }
}

/// Identifiers created by a command (e.g. the new track), so the UI can select them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditOutcome {
    pub created_tracks: Vec<TrackId>,
    pub created_regions: Vec<RegionId>,
    pub created_markers: Vec<MarkerId>,
}

/// Applies `cmd` to `project`. On error `project` is left untouched.
pub fn apply(project: &mut Project, cmd: &EditCommand) -> Result<EditOutcome, EditError> {
    let backup = project.clone();
    apply_with_backup(project, cmd, backup).map(|(outcome, _)| outcome)
}

/// Like [`apply`], but reuses a caller-provided copy of the original project
/// (which the undo history needs anyway) and hands it back on success.
pub(crate) fn apply_with_backup(
    project: &mut Project,
    cmd: &EditCommand,
    backup: Project,
) -> Result<(EditOutcome, Project), EditError> {
    let result = apply_inner(project, cmd).and_then(|o| {
        validate(project)?;
        Ok(o)
    });
    match result {
        Ok(outcome) => Ok((outcome, backup)),
        Err(e) => {
            *project = backup;
            Err(e)
        }
    }
}

pub(crate) fn invalid(msg: impl Into<String>) -> EditError {
    EditError::InvalidValue(msg.into())
}

fn track_mut(project: &mut Project, id: TrackId) -> Result<&mut Track, EditError> {
    project.track_mut(id).ok_or(EditError::UnknownTrack(id))
}

fn check_tracks_exist(project: &Project, ids: &[TrackId]) -> Result<(), EditError> {
    match ids.iter().find(|id| project.track(**id).is_none()) {
        Some(id) => Err(EditError::UnknownTrack(*id)),
        None => Ok(()),
    }
}

fn apply_inner(project: &mut Project, cmd: &EditCommand) -> Result<EditOutcome, EditError> {
    let mut outcome = EditOutcome::default();
    match cmd {
        EditCommand::RenameProject { name } => {
            validate_name("name", name)?;
            project.name = name.trim().to_owned();
        }
        EditCommand::SetTempo { bpm } => {
            project.transport.tempo =
                Tempo::new(*bpm).ok_or_else(|| invalid("tempo must be between 20 and 999 BPM"))?;
        }
        EditCommand::SetTimeSignature {
            numerator,
            denominator,
        } => {
            project.transport.time_signature = TimeSignature::new(*numerator, *denominator)
                .ok_or_else(|| invalid("unsupported time signature"))?;
        }
        EditCommand::SetKey { key } => project.transport.key = *key,
        EditCommand::SetLoop {
            enabled,
            start,
            end,
        } => {
            if start >= end {
                return Err(invalid("loop start must be before loop end"));
            }
            let lr = &mut project.transport.loop_range;
            lr.enabled = *enabled;
            lr.start = *start;
            lr.end = *end;
        }
        EditCommand::AddTrack { kind, name, index } => {
            if project.tracks.len() >= limits::MAX_TRACKS {
                return Err(EditError::Limit(format!(
                    "at most {} tracks",
                    limits::MAX_TRACKS
                )));
            }
            let name = match name {
                Some(n) => {
                    validate_name("name", n)?;
                    n.trim().to_owned()
                }
                None => unique_default_name(project, *kind),
            };
            let track = Track::new(*kind, name, project.next_track_color());
            outcome.created_tracks.push(track.id);
            let at = index
                .unwrap_or(project.tracks.len())
                .min(project.tracks.len());
            project.tracks.insert(at, track);
        }
        EditCommand::RemoveTracks { ids } => {
            check_tracks_exist(project, ids)?;
            project.tracks.retain(|t| !ids.contains(&t.id));
        }
        EditCommand::DuplicateTracks { ids } => {
            check_tracks_exist(project, ids)?;
            if project.tracks.len() + ids.len() > limits::MAX_TRACKS {
                return Err(EditError::Limit(format!(
                    "at most {} tracks",
                    limits::MAX_TRACKS
                )));
            }
            // Walk from the bottom so insert positions stay valid.
            let mut order: Vec<usize> = ids
                .iter()
                .filter_map(|id| project.track_index(*id))
                .collect();
            order.sort_unstable();
            order.dedup();
            for &i in order.iter().rev() {
                let mut copy = project.tracks[i].clone();
                copy.id = TrackId::new();
                copy.name = copy_name(&copy.name);
                copy.soloed = false;
                for r in &mut copy.regions {
                    r.id = RegionId::new();
                }
                outcome.created_tracks.push(copy.id);
                project.tracks.insert(i + 1, copy);
            }
            outcome.created_tracks.reverse();
        }
        EditCommand::RenameTrack { id, name } => {
            validate_name("name", name)?;
            track_mut(project, *id)?.name = name.trim().to_owned();
        }
        EditCommand::SetTrackColor { id, color } => track_mut(project, *id)?.color = *color,
        EditCommand::SetTracksMuted { ids, muted } => {
            check_tracks_exist(project, ids)?;
            for t in project.tracks.iter_mut().filter(|t| ids.contains(&t.id)) {
                t.muted = *muted;
            }
        }
        EditCommand::SetTracksSoloed { ids, soloed } => {
            check_tracks_exist(project, ids)?;
            for t in project.tracks.iter_mut().filter(|t| ids.contains(&t.id)) {
                t.soloed = *soloed;
            }
        }
        EditCommand::SetTrackVolume { id, volume_db } => {
            if !(Track::MIN_VOLUME_DB..=Track::MAX_VOLUME_DB).contains(volume_db) {
                return Err(invalid("volume out of range"));
            }
            track_mut(project, *id)?.volume_db = *volume_db;
        }
        EditCommand::SetTrackPan { id, pan } => {
            if !(-1.0..=1.0).contains(pan) {
                return Err(invalid("pan out of range"));
            }
            track_mut(project, *id)?.pan = *pan;
        }
        EditCommand::MoveTrack { id, to_index } => {
            let from = project
                .track_index(*id)
                .ok_or(EditError::UnknownTrack(*id))?;
            let track = project.tracks.remove(from);
            let to = (*to_index).min(project.tracks.len());
            project.tracks.insert(to, track);
        }
        EditCommand::CreateMidiRegion {
            track_id,
            start,
            length,
        } => {
            if *length == 0 {
                return Err(invalid("region length must be greater than zero"));
            }
            let track = track_mut(project, *track_id)?;
            if track.kind == TrackKind::Audio {
                return Err(EditError::Incompatible(
                    "MIDI regions can only be created on instrument or MIDI tracks".into(),
                ));
            }
            if track.regions.len() >= limits::MAX_REGIONS_PER_TRACK {
                return Err(EditError::Limit("too many regions on this track".into()));
            }
            let region = Region {
                id: RegionId::new(),
                name: track.name.clone(),
                start: *start,
                length: *length,
                muted: false,
                content: RegionContent::Midi { notes: Vec::new() },
            };
            outcome.created_regions.push(region.id);
            track.regions.push(region);
            sort_regions(track);
            extend_length_to_content(project);
        }
        EditCommand::MoveRegions {
            ids,
            delta_ticks,
            track_offset,
        } => {
            move_regions(project, ids, *delta_ticks, *track_offset)?;
            extend_length_to_content(project);
        }
        EditCommand::RemoveRegions { ids } => {
            for id in ids {
                project
                    .find_region(*id)
                    .ok_or(EditError::UnknownRegion(*id))?;
            }
            for t in &mut project.tracks {
                t.regions.retain(|r| !ids.contains(&r.id));
            }
        }
        EditCommand::RenameRegion { id, name } => {
            validate_name("name", name)?;
            let (ti, ri) = project
                .find_region(*id)
                .ok_or(EditError::UnknownRegion(*id))?;
            project.tracks[ti].regions[ri].name = name.trim().to_owned();
        }
        EditCommand::SetSampleRate { sample_rate } => {
            if !Project::SAMPLE_RATES.contains(sample_rate) {
                return Err(invalid("unsupported sample rate"));
            }
            project.sample_rate = *sample_rate;
        }
        EditCommand::SetTempoChanges { changes } => {
            project.transport.tempo_changes = changes.clone();
            project
                .transport
                .tempo_map()
                .map_err(|e| invalid(e.to_string()))?;
        }
        EditCommand::SetMeterChanges { changes } => {
            project.transport.meter_changes = changes.clone();
            project
                .transport
                .tempo_map()
                .map_err(|e| invalid(e.to_string()))?;
            extend_length_to_content(project);
        }
        EditCommand::SetMetronome { settings } => {
            if !(Track::MIN_VOLUME_DB..=Track::MAX_VOLUME_DB).contains(&settings.volume_db) {
                return Err(invalid("metronome volume out of range"));
            }
            project.transport.metronome = *settings;
        }
        EditCommand::SetCountIn { bars } => project.transport.count_in_bars = *bars,
        EditCommand::SetPreRoll { bars } => project.transport.pre_roll_bars = *bars,
        EditCommand::SetPunch {
            enabled,
            start,
            end,
        } => {
            if start >= end {
                return Err(invalid("punch-in must be before punch-out"));
            }
            let punch = &mut project.transport.punch;
            punch.enabled = *enabled;
            punch.start = *start;
            punch.end = *end;
        }
        EditCommand::AddMarker { tick, name } => {
            outcome
                .created_markers
                .push(tl::add_marker(project, *tick, name.as_deref())?);
        }
        EditCommand::UpdateMarker { id, name, tick } => {
            tl::update_marker(project, *id, name, *tick)?;
        }
        EditCommand::RemoveMarkers { ids } => tl::remove_markers(project, ids)?,
        EditCommand::ImportAudio { assets, placements } => {
            tl::import_audio(project, assets, placements, &mut outcome)?;
            extend_length_to_content(project);
        }
        EditCommand::RelinkMedia { id, path, info } => {
            let asset = project
                .media
                .iter_mut()
                .find(|m| m.id == *id)
                .ok_or_else(|| invalid("unknown media file"))?;
            asset.path = path.clone();
            asset.info = info.clone();
        }
        EditCommand::SplitRegions { ids, at } => {
            tl::split_regions(project, ids, *at, &mut outcome)?;
        }
        EditCommand::TrimRegion { id, start, end } => {
            tl::trim_region(project, *id, *start, *end)?;
            extend_length_to_content(project);
        }
        EditCommand::DuplicateRegions { ids } => {
            tl::duplicate_regions(project, ids, &mut outcome)?;
            extend_length_to_content(project);
        }
        EditCommand::PasteRegions {
            track_id,
            at,
            items,
        } => {
            tl::paste_regions(project, *track_id, *at, items, &mut outcome)?;
            extend_length_to_content(project);
        }
        EditCommand::SetRegionsMuted { ids, muted } => {
            for id in ids {
                let (ti, ri) = project
                    .find_region(*id)
                    .ok_or(EditError::UnknownRegion(*id))?;
                project.tracks[ti].regions[ri].muted = *muted;
            }
        }
        EditCommand::SetAudioRegion {
            id,
            gain_db,
            fade_in,
            fade_out,
            looped,
        } => {
            tl::set_audio_region(project, *id, *gain_db, *fade_in, *fade_out, *looped)?;
        }
    }
    Ok(outcome)
}

fn move_regions(
    project: &mut Project,
    ids: &[RegionId],
    delta: i64,
    track_offset: i32,
) -> Result<(), EditError> {
    let track_count = project.tracks.len() as i64;
    let mut moving = Vec::with_capacity(ids.len());
    for id in ids {
        let (ti, ri) = project
            .find_region(*id)
            .ok_or(EditError::UnknownRegion(*id))?;
        let target = ti as i64 + i64::from(track_offset);
        if !(0..track_count).contains(&target) {
            return Err(EditError::Incompatible("there is no track there".into()));
        }
        let target = target as usize;
        let region = &project.tracks[ti].regions[ri];
        if !project.tracks[target].kind.accepts(&region.content) {
            return Err(EditError::Incompatible(format!(
                "a {} region cannot be placed on a {} track",
                match region.content {
                    RegionContent::Audio { .. } => "audio",
                    RegionContent::Midi { .. } => "MIDI",
                },
                project.tracks[target].kind.default_name().to_lowercase()
            )));
        }
        moving.push((ti, ri, target));
    }
    // The earliest selected region clamps at zero; the group keeps its shape.
    let earliest = moving
        .iter()
        .map(|&(ti, ri, _)| project.tracks[ti].regions[ri].start)
        .min()
        .unwrap_or(0);
    let delta = delta.max(-(earliest.min(i64::MAX as u64) as i64));

    // Remove from the bottom-right so indices stay valid, then re-insert.
    moving.sort_unstable_by_key(|m| std::cmp::Reverse((m.0, m.1)));
    let mut extracted = Vec::with_capacity(moving.len());
    for (ti, ri, target) in moving {
        let mut region = project.tracks[ti].regions.remove(ri);
        region.start = region
            .start
            .checked_add_signed(delta)
            .filter(|s| *s <= limits::MAX_TICKS)
            .ok_or_else(|| invalid("region would move out of range"))?;
        extracted.push((target, region));
    }
    for (target, region) in extracted {
        project.tracks[target].regions.push(region);
    }
    for t in &mut project.tracks {
        if t.regions.len() > limits::MAX_REGIONS_PER_TRACK {
            return Err(EditError::Limit("too many regions on this track".into()));
        }
        sort_regions(t);
    }
    Ok(())
}

pub(crate) fn sort_regions(track: &mut Track) {
    track.regions.sort_by_key(|r| r.start);
}

/// Grows the song so the last region ends at least two bars before the end.
fn extend_length_to_content(project: &mut Project) {
    let end = project.content_end();
    let needed = match project.transport.tempo_map() {
        Ok(map) => {
            let bar = map.bar_at(end);
            bar.bar + u64::from(end > bar.start) + 2
        }
        Err(_) => {
            let per_bar = project.transport.time_signature.ticks_per_bar();
            end.div_ceil(per_bar) + 2
        }
    };
    let needed = u32::try_from(needed)
        .unwrap_or(u32::MAX)
        .min(limits::MAX_LENGTH_BARS);
    project.length_bars = project.length_bars.max(needed);
}

fn unique_default_name(project: &Project, kind: TrackKind) -> String {
    let base = kind.default_name();
    let n = project.tracks.iter().filter(|t| t.kind == kind).count() + 1;
    let mut i = n;
    loop {
        let candidate = format!("{base} {i}");
        if !project.tracks.iter().any(|t| t.name == candidate) {
            return candidate;
        }
        i += 1;
    }
}

fn copy_name(name: &str) -> String {
    let suffix = " copy";
    let max = limits::MAX_NAME_CHARS - suffix.len();
    let base: String = name.chars().take(max).collect();
    format!("{base}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MidiNote, TRACK_PALETTE};

    fn add(project: &mut Project, kind: TrackKind) -> TrackId {
        apply(
            project,
            &EditCommand::AddTrack {
                kind,
                name: None,
                index: None,
            },
        )
        .unwrap()
        .created_tracks[0]
    }

    fn midi_region(project: &mut Project, track: TrackId, start: Ticks) -> RegionId {
        apply(
            project,
            &EditCommand::CreateMidiRegion {
                track_id: track,
                start,
                length: 3840,
            },
        )
        .unwrap()
        .created_regions[0]
    }

    #[test]
    fn add_track_names_and_colours() {
        let mut p = Project::new("S");
        let a = add(&mut p, TrackKind::Audio);
        let b = add(&mut p, TrackKind::Audio);
        let c = add(&mut p, TrackKind::Instrument);
        assert_eq!(p.track(a).unwrap().name, "Audio 1");
        assert_eq!(p.track(b).unwrap().name, "Audio 2");
        assert_eq!(p.track(c).unwrap().name, "Instrument 1");
        assert_eq!(p.track(a).unwrap().color, TRACK_PALETTE[0]);
        assert_eq!(p.track(b).unwrap().color, TRACK_PALETTE[1]);
    }

    #[test]
    fn add_track_at_index() {
        let mut p = Project::new("S");
        add(&mut p, TrackKind::Audio);
        let id = apply(
            &mut p,
            &EditCommand::AddTrack {
                kind: TrackKind::Midi,
                name: Some(" Synth ".into()),
                index: Some(0),
            },
        )
        .unwrap()
        .created_tracks[0];
        assert_eq!(p.tracks[0].id, id);
        assert_eq!(p.tracks[0].name, "Synth");
    }

    #[test]
    fn remove_tracks_is_all_or_nothing() {
        let mut p = Project::new("S");
        let a = add(&mut p, TrackKind::Audio);
        let err = apply(
            &mut p,
            &EditCommand::RemoveTracks {
                ids: vec![a, TrackId::new()],
            },
        );
        assert!(matches!(err, Err(EditError::UnknownTrack(_))));
        assert_eq!(p.tracks.len(), 1);
        apply(&mut p, &EditCommand::RemoveTracks { ids: vec![a] }).unwrap();
        assert!(p.tracks.is_empty());
    }

    #[test]
    fn duplicate_tracks_gets_fresh_ids_and_follows_original() {
        let mut p = Project::new("S");
        let a = add(&mut p, TrackKind::Instrument);
        let b = add(&mut p, TrackKind::Audio);
        let r = midi_region(&mut p, a, 0);
        let out = apply(&mut p, &EditCommand::DuplicateTracks { ids: vec![b, a] }).unwrap();
        let names: Vec<_> = p.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Instrument 1",
                "Instrument 1 copy",
                "Audio 1",
                "Audio 1 copy"
            ]
        );
        assert_eq!(out.created_tracks, vec![p.tracks[1].id, p.tracks[3].id]);
        assert_ne!(p.tracks[1].regions[0].id, r);
        validate(&p).unwrap();
    }

    #[test]
    fn rename_validates() {
        let mut p = Project::new("S");
        let a = add(&mut p, TrackKind::Audio);
        assert!(
            apply(
                &mut p,
                &EditCommand::RenameTrack {
                    id: a,
                    name: "   ".into()
                }
            )
            .is_err()
        );
        apply(
            &mut p,
            &EditCommand::RenameTrack {
                id: a,
                name: "Vox".into(),
            },
        )
        .unwrap();
        assert_eq!(p.tracks[0].name, "Vox");
    }

    #[test]
    fn mute_and_solo_many() {
        let mut p = Project::new("S");
        let a = add(&mut p, TrackKind::Audio);
        let b = add(&mut p, TrackKind::Audio);
        apply(
            &mut p,
            &EditCommand::SetTracksSoloed {
                ids: vec![a],
                soloed: true,
            },
        )
        .unwrap();
        assert!(p.any_solo());
        assert!(p.track(a).unwrap().is_audible(true));
        assert!(!p.track(b).unwrap().is_audible(true));
        apply(
            &mut p,
            &EditCommand::SetTracksMuted {
                ids: vec![a, b],
                muted: true,
            },
        )
        .unwrap();
        assert!(p.tracks.iter().all(|t| t.muted));
    }

    #[test]
    fn transport_edits_are_validated() {
        let mut p = Project::new("S");
        assert!(apply(&mut p, &EditCommand::SetTempo { bpm: 5.0 }).is_err());
        apply(&mut p, &EditCommand::SetTempo { bpm: 96.5 }).unwrap();
        assert_eq!(p.transport.tempo.bpm(), 96.5);
        assert!(
            apply(
                &mut p,
                &EditCommand::SetTimeSignature {
                    numerator: 5,
                    denominator: 3
                }
            )
            .is_err()
        );
        apply(
            &mut p,
            &EditCommand::SetTimeSignature {
                numerator: 6,
                denominator: 8,
            },
        )
        .unwrap();
        assert!(
            apply(
                &mut p,
                &EditCommand::SetLoop {
                    enabled: true,
                    start: 10,
                    end: 10
                }
            )
            .is_err()
        );
        assert!(
            apply(
                &mut p,
                &EditCommand::SetTrackVolume {
                    id: TrackId::new(),
                    volume_db: 0.0
                }
            )
            .is_err()
        );
    }

    #[test]
    fn move_regions_in_time_and_across_tracks() {
        let mut p = Project::new("S");
        let keys = add(&mut p, TrackKind::Instrument);
        let _synth = add(&mut p, TrackKind::Midi);
        let audio = add(&mut p, TrackKind::Audio);
        let r1 = midi_region(&mut p, keys, 3840);
        let r2 = midi_region(&mut p, keys, 7680);

        // Group clamps at zero, keeping relative spacing.
        apply(
            &mut p,
            &EditCommand::MoveRegions {
                ids: vec![r1, r2],
                delta_ticks: -10_000,
                track_offset: 1,
            },
        )
        .unwrap();
        let starts: Vec<_> = p.tracks[1].regions.iter().map(|r| r.start).collect();
        assert_eq!(starts, [0, 3840]);
        assert!(p.tracks[0].regions.is_empty());

        // MIDI cannot land on the audio track; nothing changes.
        let before = p.clone();
        let err = apply(
            &mut p,
            &EditCommand::MoveRegions {
                ids: vec![r1],
                delta_ticks: 0,
                track_offset: 1,
            },
        );
        assert!(matches!(err, Err(EditError::Incompatible(_))));
        assert_eq!(p, before);
        assert_eq!(p.tracks[2].id, audio);

        // Off the end of the track list.
        assert!(
            apply(
                &mut p,
                &EditCommand::MoveRegions {
                    ids: vec![r1],
                    delta_ticks: 0,
                    track_offset: 5
                }
            )
            .is_err()
        );
    }

    #[test]
    fn moving_regions_extends_song_length() {
        let mut p = Project::new("S");
        let keys = add(&mut p, TrackKind::Instrument);
        let r = midi_region(&mut p, keys, 0);
        apply(
            &mut p,
            &EditCommand::MoveRegions {
                ids: vec![r],
                delta_ticks: 3840 * 100,
                track_offset: 0,
            },
        )
        .unwrap();
        assert!(p.length_bars >= 103);
    }

    #[test]
    fn midi_regions_only_on_midi_capable_tracks() {
        let mut p = Project::new("S");
        let a = add(&mut p, TrackKind::Audio);
        let err = apply(
            &mut p,
            &EditCommand::CreateMidiRegion {
                track_id: a,
                start: 0,
                length: 960,
            },
        );
        assert!(matches!(err, Err(EditError::Incompatible(_))));
    }

    #[test]
    fn remove_and_rename_regions() {
        let mut p = Project::new("S");
        let k = add(&mut p, TrackKind::Instrument);
        let r = midi_region(&mut p, k, 0);
        if let RegionContent::Midi { notes } = &mut p.tracks[0].regions[0].content {
            notes.push(MidiNote {
                pitch: 60,
                velocity: 100,
                start: 0,
                length: 480,
            });
        }
        apply(
            &mut p,
            &EditCommand::RenameRegion {
                id: r,
                name: "Chords".into(),
            },
        )
        .unwrap();
        assert_eq!(p.tracks[0].regions[0].name, "Chords");
        apply(&mut p, &EditCommand::RemoveRegions { ids: vec![r] }).unwrap();
        assert_eq!(p.region_count(), 0);
        assert!(apply(&mut p, &EditCommand::RemoveRegions { ids: vec![r] }).is_err());
    }

    #[test]
    fn commands_deserialize_from_ui_json() {
        let json = r#"{"type":"setTrackVolume","id":"7b6f3f0e-3ad4-4a0d-9d0b-3c3cfa2a1f10","volumeDb":-6.5}"#;
        let cmd: EditCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, EditCommand::SetTrackVolume { volume_db, .. } if volume_db == -6.5));
        let json = r#"{"type":"addTrack","kind":"instrument"}"#;
        assert!(serde_json::from_str::<EditCommand>(json).is_ok());
        assert!(serde_json::from_str::<EditCommand>(r#"{"type":"formatDisk"}"#).is_err());
    }
}
