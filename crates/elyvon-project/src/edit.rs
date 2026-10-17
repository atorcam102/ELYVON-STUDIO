//! Edit commands: the only way the UI changes a project.
//!
//! Each command is applied atomically by [`apply`]: on error the caller's
//! project is restored, so a half-applied edit can never be observed.
//! Continuous gestures (dragging a fader or a region) are previewed in the UI
//! and committed as a single command on release, giving one undo step each.

use elyvon_core::{Color, Key, MeterChange, Tempo, TempoChange, Ticks, TimeSignature};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::{
    self as au, AutomationMode, AutomationPoint, AutomationTarget, AutomationWrite, PointSpec,
};
use crate::error::EditError;
use crate::model::FadeCurve;
use crate::region_edit::{self as re, MediaRelink, RegionGain};
use crate::midi_edit::{
    self as me, ControllerLane, LanePoint, NoteEdge, NoteSpec, NoteTarget, NoteTransform,
};
use crate::mixer::{
    AuxId, ChannelRef, FolderId, GroupId, InsertId, MixGroup, PanLaw, Processor, Send, TrackFolder,
};
use crate::mixer_edit::{self as mx, Linked, StripPatch};
use crate::model::{
    MarkerId, MediaAsset, MediaId, MediaInfo, MetronomeSettings, MidiNote, Project, Region,
    RegionContent, RegionId, Track, TrackId, TrackKind,
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
    ImportMidi {
        regions: Vec<crate::io::MidiImportRegion>,
        tempo: Option<Tempo>,
        tempo_changes: Vec<TempoChange>,
        time_signature: Option<TimeSignature>,
        meter_changes: Vec<MeterChange>,
    },
    ImportTakes {
        assets: Vec<MediaAsset>,
        placements: Vec<AudioPlacement>,
        #[ts(type = "number")]
        recorded_at: u64,
    },
    SelectTake {
        track_id: TrackId,
        folder: RegionId,
        take: u32,
    },
    SetTrackIo {
        id: TrackId,
        io: crate::io::TrackIo,
    },
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
    // ---- Piano roll (Phase 08) ----
    AddNotes {
        region_id: RegionId,
        notes: Vec<NoteSpec>,
    },
    RemoveNotes {
        region_id: RegionId,
        ids: Vec<u32>,
    },
    /// Replaces the listed notes (matched by id) field by field.
    UpdateNotes {
        region_id: RegionId,
        notes: Vec<MidiNote>,
    },
    /// Moves notes in time and pitch as a group; `copy` leaves the originals.
    MoveNotes {
        region_id: RegionId,
        ids: Vec<u32>,
        #[ts(type = "number")]
        delta_ticks: i64,
        delta_pitch: i32,
        copy: bool,
    },
    ResizeNotes {
        region_id: RegionId,
        ids: Vec<u32>,
        edge: NoteEdge,
        #[ts(type = "number")]
        delta: i64,
    },
    TransformNotes {
        targets: Vec<NoteTarget>,
        transform: NoteTransform,
    },
    SplitChordVoices {
        region_id: RegionId,
        #[ts(optional)]
        ids: Option<Vec<u32>>,
    },
    /// Replaces a controller lane's events in `[from, to)` with `points`.
    EditControllerLane {
        region_id: RegionId,
        lane: ControllerLane,
        #[ts(type = "number")]
        from: Ticks,
        #[ts(type = "number")]
        to: Ticks,
        points: Vec<LanePoint>,
    },
    /// Several edits as one undo step (multi-region gestures). Not nestable.
    Batch {
        commands: Vec<EditCommand>,
    },
    // ---- Mixer (Phase 09) ----
    SetStrip {
        channel: ChannelRef,
        patch: StripPatch,
    },
    /// Adds or changes (`send`) or removes (no `send`) a send.
    SetSend {
        channel: ChannelRef,
        target: AuxId,
        #[ts(optional)]
        send: Option<Send>,
    },
    AddInsert {
        channel: ChannelRef,
        processor: Processor,
        #[ts(optional, type = "number")]
        index: Option<usize>,
    },
    RemoveInsert {
        channel: ChannelRef,
        id: InsertId,
    },
    MoveInsert {
        channel: ChannelRef,
        id: InsertId,
        #[ts(type = "number")]
        to_index: usize,
    },
    SetInsert {
        channel: ChannelRef,
        id: InsertId,
        bypass: bool,
        processor: Processor,
    },
    AddAux {
        #[ts(optional)]
        name: Option<String>,
        /// Channels that get a post-fader send to the new aux.
        sources: Vec<ChannelRef>,
    },
    RemoveAuxes {
        ids: Vec<AuxId>,
    },
    AddGroup {
        #[ts(optional)]
        name: Option<String>,
        members: Vec<ChannelRef>,
    },
    UpdateGroup {
        group: MixGroup,
    },
    RemoveGroup {
        id: GroupId,
    },
    SetGroupMembership {
        channels: Vec<ChannelRef>,
        #[ts(optional)]
        group: Option<GroupId>,
    },
    AddFolder {
        #[ts(optional)]
        name: Option<String>,
        tracks: Vec<TrackId>,
    },
    UpdateFolder {
        folder: TrackFolder,
    },
    RemoveFolder {
        id: FolderId,
    },
    SetTrackFolder {
        ids: Vec<TrackId>,
        #[ts(optional)]
        folder: Option<FolderId>,
    },
    SetPanLaw {
        law: PanLaw,
    },
    // ---- Automation (Phase 10) ----
    SetAutomationMode {
        channel: ChannelRef,
        mode: AutomationMode,
    },
    /// Adds points (creating the lane when needed).
    AddAutomationPoints {
        target: AutomationTarget,
        points: Vec<PointSpec>,
    },
    RemoveAutomationPoints {
        target: AutomationTarget,
        ids: Vec<u32>,
    },
    /// Replaces the listed points (matched by id).
    UpdateAutomationPoints {
        target: AutomationTarget,
        points: Vec<AutomationPoint>,
    },
    /// Moves points in time and by a fraction of the value range; `copy`
    /// leaves the originals (duplicate).
    MoveAutomationPoints {
        target: AutomationTarget,
        ids: Vec<u32>,
        #[ts(type = "number")]
        delta_ticks: i64,
        delta_value: f64,
        copy: bool,
    },
    RemoveAutomationLane {
        target: AutomationTarget,
    },
    /// Removes points within `tolerance` (fraction of the range) of a line.
    SimplifyAutomation {
        target: AutomationTarget,
        tolerance: f64,
    },
    /// Range replacements captured during playback (one undo step).
    WriteAutomation {
        writes: Vec<AutomationWrite>,
    },
    // ---- Non-destructive editing, takes and comping (Phase 11) ----
    /// Moves the audio (or notes) inside regions without moving the regions.
    SlipRegions {
        ids: Vec<RegionId>,
        #[ts(type = "number")]
        delta_ticks: i64,
    },
    /// Deletes regions and closes the gaps on their tracks.
    RippleDelete {
        ids: Vec<RegionId>,
    },
    /// Marquee: splits the tracks' regions at the range edges and reports
    /// the regions inside (`selectedRegions`).
    SplitRange {
        track_ids: Vec<TrackId>,
        #[ts(type = "number")]
        start: Ticks,
        #[ts(type = "number")]
        end: Ticks,
    },
    /// Marquee delete, optionally closing the gap.
    CutRange {
        track_ids: Vec<TrackId>,
        #[ts(type = "number")]
        start: Ticks,
        #[ts(type = "number")]
        end: Ticks,
        ripple: bool,
    },
    GlueRegions {
        ids: Vec<RegionId>,
    },
    CreateCrossfade {
        left: RegionId,
        right: RegionId,
        length_ms: f64,
    },
    SetRegionsReversed {
        ids: Vec<RegionId>,
        reversed: bool,
    },
    SetFadeCurve {
        ids: Vec<RegionId>,
        curve: FadeCurve,
    },
    /// Splits a region at several positions (e.g. detected transients).
    SliceRegion {
        id: RegionId,
        #[ts(type = "Array<number>")]
        positions: Vec<Ticks>,
    },
    /// Region gains measured by the engine's normaliser.
    NormalizeRegions {
        gains: Vec<RegionGain>,
    },
    /// Plays `[start, end)` of a take folder from `take`.
    CompTake {
        track_id: TrackId,
        folder: RegionId,
        take: u32,
        #[ts(type = "number")]
        start: Ticks,
        #[ts(type = "number")]
        end: Ticks,
    },
    SetCompCrossfade {
        track_id: TrackId,
        folder: RegionId,
        crossfade_ms: f64,
    },
    /// Consolidates a comp into ordinary regions (takes are kept).
    FlattenComp {
        track_id: TrackId,
        folder: RegionId,
    },
    UnflattenComp {
        track_id: TrackId,
        folder: RegionId,
    },
    /// Plays a pre-rendered file (rendered by the shell) instead of the
    /// track's regions and inserts.
    FreezeTrack {
        id: TrackId,
        media: MediaAsset,
        #[ts(type = "number")]
        start: Ticks,
    },
    UnfreezeTrack {
        id: TrackId,
    },
    /// Places a rendered file on a new track below `source_track`.
    BounceInPlace {
        asset: MediaAsset,
        #[ts(type = "number")]
        start: Ticks,
        source_track: TrackId,
        regions: Vec<RegionId>,
        mute_sources: bool,
    },
    /// Relinks several pool entries at once (repair, collect media).
    RelinkMediaFiles {
        items: Vec<MediaRelink>,
    },
    /// Drops pool entries no region uses (the files stay on disk).
    RemoveUnusedMedia,
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
            Self::ImportMidi { .. } => "Import / Record MIDI",
            Self::ImportTakes { .. } => "Record Audio",
            Self::SelectTake { .. } => "Select Take",
            Self::SetTrackIo { .. } => "Change Track Input",
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
            Self::AddNotes { .. } => "Add Notes",
            Self::RemoveNotes { .. } => "Delete Notes",
            Self::UpdateNotes { .. } => "Edit Notes",
            Self::MoveNotes { copy: true, .. } => "Copy Notes",
            Self::MoveNotes { .. } => "Move Notes",
            Self::ResizeNotes { .. } => "Resize Notes",
            Self::TransformNotes { transform, .. } => match transform {
                NoteTransform::Quantize { .. } => "Quantize",
                NoteTransform::Unquantize => "Unquantize",
                NoteTransform::Humanize { .. } => "Humanize",
                NoteTransform::Transpose { .. } => "Transpose",
                NoteTransform::Legato { .. } => "Legato",
                NoteTransform::Velocity { .. } => "Change Velocity",
                NoteTransform::Mute { .. } => "Mute Notes",
                NoteTransform::Channel { .. } => "Change MIDI Channel",
                NoteTransform::FixedLength { .. } => "Set Note Length",
            },
            Self::SplitChordVoices { .. } => "Split Chord Voices",
            Self::EditControllerLane { .. } => "Edit Controller Lane",
            Self::Batch { commands } => match commands.first().map(EditCommand::label) {
                Some(first) if commands.iter().all(|c| c.label() == first) => first,
                _ => "Multiple Edits",
            },
            Self::SetStrip { .. } => "Change Channel Strip",
            Self::SetSend { send: None, .. } => "Remove Send",
            Self::SetSend { .. } => "Change Send",
            Self::AddInsert { .. } => "Add Insert",
            Self::RemoveInsert { .. } => "Remove Insert",
            Self::MoveInsert { .. } => "Reorder Inserts",
            Self::SetInsert { .. } => "Change Insert",
            Self::AddAux { .. } => "New Auxiliary Channel",
            Self::RemoveAuxes { .. } => "Delete Auxiliary Channels",
            Self::AddGroup { .. } => "New Group",
            Self::UpdateGroup { .. } => "Change Group",
            Self::RemoveGroup { .. } => "Delete Group",
            Self::SetGroupMembership { .. } => "Assign Group",
            Self::AddFolder { .. } => "New Folder",
            Self::UpdateFolder { .. } => "Change Folder",
            Self::RemoveFolder { .. } => "Delete Folder",
            Self::SetTrackFolder { .. } => "Move to Folder",
            Self::SetPanLaw { .. } => "Change Pan Law",
            Self::SetAutomationMode { .. } => "Change Automation Mode",
            Self::AddAutomationPoints { .. } => "Add Automation",
            Self::RemoveAutomationPoints { .. } => "Delete Automation",
            Self::UpdateAutomationPoints { .. } => "Edit Automation",
            Self::MoveAutomationPoints { copy: true, .. } => "Copy Automation",
            Self::MoveAutomationPoints { .. } => "Move Automation",
            Self::RemoveAutomationLane { .. } => "Delete Automation Lane",
            Self::SimplifyAutomation { .. } => "Simplify Automation",
            Self::WriteAutomation { .. } => "Write Automation",
            Self::SlipRegions { .. } => "Slip Regions",
            Self::RippleDelete { .. } => "Ripple Delete",
            Self::SplitRange { .. } => "Split at Range",
            Self::CutRange { ripple: true, .. } => "Ripple Delete Range",
            Self::CutRange { .. } => "Delete Range",
            Self::GlueRegions { .. } => "Glue Regions",
            Self::CreateCrossfade { .. } => "Crossfade",
            Self::SetRegionsReversed { .. } => "Reverse",
            Self::SetFadeCurve { .. } => "Change Fade Shape",
            Self::SliceRegion { .. } => "Slice Region",
            Self::NormalizeRegions { .. } => "Normalize",
            Self::CompTake { .. } => "Comp Take",
            Self::SetCompCrossfade { .. } => "Comp Crossfade",
            Self::FlattenComp { .. } => "Flatten Comp",
            Self::UnflattenComp { .. } => "Unflatten Comp",
            Self::FreezeTrack { .. } => "Freeze Track",
            Self::UnfreezeTrack { .. } => "Unfreeze Track",
            Self::BounceInPlace { .. } => "Bounce in Place",
            Self::RelinkMediaFiles { .. } => "Relink Audio Files",
            Self::RemoveUnusedMedia => "Remove Unused Audio Files",
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
    /// Note ids, within the edited region.
    pub created_notes: Vec<u32>,
    pub created_auxes: Vec<AuxId>,
    pub created_inserts: Vec<InsertId>,
    pub created_groups: Vec<GroupId>,
    pub created_folders: Vec<FolderId>,
    /// Automation point ids, within the edited lane.
    pub created_points: Vec<u32>,
    /// Regions the UI should select (e.g. inside a marquee range).
    pub selected_regions: Vec<RegionId>,
}

impl EditOutcome {
    fn merge(&mut self, other: EditOutcome) {
        self.created_points.extend(other.created_points);
        self.selected_regions.extend(other.selected_regions);
        self.created_tracks.extend(other.created_tracks);
        self.created_regions.extend(other.created_regions);
        self.created_markers.extend(other.created_markers);
        self.created_notes.extend(other.created_notes);
        self.created_auxes.extend(other.created_auxes);
        self.created_inserts.extend(other.created_inserts);
        self.created_groups.extend(other.created_groups);
        self.created_folders.extend(other.created_folders);
    }
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
        settle(project);
        crate::comp::check_frozen(&backup, project)?;
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

/// Keeps derived data consistent after an edit: automation of removed sends
/// and inserts, comps of deleted takes.
fn settle(project: &mut Project) {
    au::prune(project);
    crate::comp::prune(project);
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
        EditCommand::SetTrackIo { id, io } => {
            io.validate()
                .map_err(|e| EditError::InvalidValue(e.into()))?;
            track_mut(project, *id)?.io = io.clone();
        }
        EditCommand::SetTrackColor { id, color } => track_mut(project, *id)?.color = *color,
        // Mute, solo, volume and pan follow linked mixer groups.
        EditCommand::SetTracksMuted { ids, muted } => {
            check_tracks_exist(project, ids)?;
            let channels: Vec<_> = ids.iter().map(|&id| ChannelRef::Track { id }).collect();
            mx::set_flag(project, &channels, Linked::Mute, *muted)?;
        }
        EditCommand::SetTracksSoloed { ids, soloed } => {
            check_tracks_exist(project, ids)?;
            let channels: Vec<_> = ids.iter().map(|&id| ChannelRef::Track { id }).collect();
            mx::set_flag(project, &channels, Linked::Solo, *soloed)?;
        }
        EditCommand::SetTrackVolume { id, volume_db } => {
            if !(Track::MIN_VOLUME_DB..=Track::MAX_VOLUME_DB).contains(volume_db) {
                return Err(invalid("volume out of range"));
            }
            track_mut(project, *id)?;
            mx::set_volume(project, ChannelRef::Track { id: *id }, *volume_db)?;
        }
        EditCommand::SetTrackPan { id, pan } => {
            if !(-1.0..=1.0).contains(pan) {
                return Err(invalid("pan out of range"));
            }
            track_mut(project, *id)?;
            mx::set_pan(project, ChannelRef::Track { id: *id }, *pan)?;
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
                capture: None,
                midi_events: Vec::new(),
                id: RegionId::new(),
                name: track.name.clone(),
                start: *start,
                length: *length,
                muted: false,
                content: RegionContent::Midi { notes: Vec::new() },
                automation: Vec::new(),
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
            // Phase-locked partners follow moves in time (not across tracks).
            let ids = if *track_offset == 0 {
                re::phase_locked(project, ids)
            } else {
                ids.clone()
            };
            move_regions(project, &ids, *delta_ticks, *track_offset)?;
            extend_length_to_content(project);
        }
        EditCommand::RemoveRegions { ids } => {
            let ids = &re::phase_locked(project, ids);
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
        EditCommand::ImportMidi {
            regions,
            tempo,
            tempo_changes,
            time_signature,
            meter_changes,
        } => {
            if regions.is_empty() || regions.len() > 512 {
                return Err(invalid("choose 1–512 MIDI tracks"));
            }
            if let Some(tempo) = tempo {
                project.transport.tempo = *tempo;
                project.transport.tempo_changes = tempo_changes.clone();
            }
            if let Some(ts) = time_signature {
                project.transport.time_signature = *ts;
                project.transport.meter_changes = meter_changes.clone();
            }
            for source in regions {
                validate_name("MIDI region", &source.name)?;
                let id = match source.track_id {
                    Some(id) => id,
                    None => {
                        if project.tracks.len() >= limits::MAX_TRACKS {
                            return Err(EditError::Limit("too many tracks".into()));
                        }
                        let track = Track::new(
                            TrackKind::Midi,
                            &source.name,
                            crate::model::TRACK_PALETTE
                                [project.tracks.len() % crate::model::TRACK_PALETTE.len()],
                        );
                        let id = track.id;
                        project.tracks.push(track);
                        outcome.created_tracks.push(id);
                        id
                    }
                };
                let track = track_mut(project, id)?;
                if track.kind == TrackKind::Audio {
                    return Err(invalid("MIDI requires a MIDI or instrument track"));
                }
                let length = source.length.max(1);
                // Note on/off pairs become editable notes; controllers,
                // pressure, bend, program and SysEx stay as events.
                let (notes, events) = if source.events.iter().any(|e| me::is_note_message(&e.bytes))
                {
                    me::split_note_events(&source.events, length)
                } else {
                    (me::with_ids(source.notes.clone()), source.events.clone())
                };
                let region = Region {
                    capture: None,
                    midi_events: events,
                    id: RegionId::new(),
                    name: source.name.clone(),
                    start: source.start,
                    length,
                    muted: false,
                    content: RegionContent::Midi { notes },
                    automation: Vec::new(),
                };
                outcome.created_regions.push(region.id);
                track.regions.push(region);
            }
            extend_length_to_content(project);
        }
        EditCommand::ImportTakes {
            assets,
            placements,
            recorded_at,
        } => {
            tl::import_audio(project, assets, placements, &mut outcome)?;
            for track in &mut project.tracks {
                let folder = RegionId::new();
                let mut take = 0;
                for region in &mut track.regions {
                    if outcome.created_regions.contains(&region.id) {
                        take += 1;
                        region.capture = Some(crate::io::CaptureMetadata {
                            folder,
                            take,
                            recorded_at: *recorded_at,
                        });
                    }
                }
                for region in &mut track.regions {
                    if let Some(c) = &region.capture
                        && c.folder == folder
                    {
                        region.muted = c.take != take;
                    }
                }
            }
            extend_length_to_content(project);
        }
        EditCommand::SelectTake {
            track_id,
            folder,
            take,
        } => crate::comp::select_take(project, *track_id, *folder, *take)?,
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
            let ids = re::phase_locked(project, ids);
            tl::split_regions(project, &ids, *at, &mut outcome)?;
        }
        EditCommand::TrimRegion { id, start, end } => {
            for id in re::phase_locked(project, &[*id]) {
                tl::trim_region(project, id, *start, *end)?;
            }
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
            for id in &re::phase_locked(project, ids) {
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
        EditCommand::AddNotes { region_id, notes } => {
            me::add_notes(project, *region_id, notes, &mut outcome)?;
            extend_length_to_content(project);
        }
        EditCommand::RemoveNotes { region_id, ids } => me::remove_notes(project, *region_id, ids)?,
        EditCommand::UpdateNotes { region_id, notes } => {
            me::update_notes(project, *region_id, notes)?;
            extend_length_to_content(project);
        }
        EditCommand::MoveNotes {
            region_id,
            ids,
            delta_ticks,
            delta_pitch,
            copy,
        } => {
            me::move_notes(
                project,
                *region_id,
                ids,
                *delta_ticks,
                *delta_pitch,
                *copy,
                &mut outcome,
            )?;
            extend_length_to_content(project);
        }
        EditCommand::ResizeNotes {
            region_id,
            ids,
            edge,
            delta,
        } => {
            me::resize_notes(project, *region_id, ids, *edge, *delta)?;
            extend_length_to_content(project);
        }
        EditCommand::TransformNotes { targets, transform } => {
            me::transform_notes(project, targets, transform)?;
            extend_length_to_content(project);
        }
        EditCommand::SplitChordVoices { region_id, ids } => {
            me::split_chord_voices(project, *region_id, ids.as_deref(), &mut outcome)?;
        }
        EditCommand::EditControllerLane {
            region_id,
            lane,
            from,
            to,
            points,
        } => me::edit_lane(project, *region_id, *lane, *from, *to, points)?,
        EditCommand::Batch { commands } => {
            if commands.is_empty() || commands.len() > 256 {
                return Err(invalid("a batch holds 1–256 edits"));
            }
            for c in commands {
                if matches!(c, EditCommand::Batch { .. }) {
                    return Err(invalid("batches cannot be nested"));
                }
                outcome.merge(apply_inner(project, c)?);
                // Later edits rely on the invariants of the earlier ones.
                settle(project);
                validate(project)?;
            }
        }
        EditCommand::SetStrip { channel, patch } => mx::set_strip(project, *channel, patch)?,
        EditCommand::SetSend {
            channel,
            target,
            send,
        } => mx::set_send(project, *channel, *target, *send)?,
        EditCommand::AddInsert {
            channel,
            processor,
            index,
        } => mx::add_insert(project, *channel, *processor, *index, &mut outcome)?,
        EditCommand::RemoveInsert { channel, id } => mx::remove_insert(project, *channel, *id)?,
        EditCommand::MoveInsert {
            channel,
            id,
            to_index,
        } => mx::move_insert(project, *channel, *id, *to_index)?,
        EditCommand::SetInsert {
            channel,
            id,
            bypass,
            processor,
        } => mx::set_insert(project, *channel, *id, *bypass, *processor)?,
        EditCommand::AddAux { name, sources } => {
            let aux = mx::add_aux(project, name.as_deref(), &mut outcome)?;
            for &source in sources {
                mx::set_send(
                    project,
                    source,
                    aux,
                    Some(Send {
                        target: aux,
                        level_db: 0.0,
                        pre_fader: false,
                        muted: false,
                    }),
                )?;
            }
        }
        EditCommand::RemoveAuxes { ids } => mx::remove_auxes(project, ids)?,
        EditCommand::AddGroup { name, members } => {
            mx::add_group(project, name.as_deref(), members, &mut outcome)?
        }
        EditCommand::UpdateGroup { group } => mx::update_group(project, group)?,
        EditCommand::RemoveGroup { id } => mx::remove_group(project, *id)?,
        EditCommand::SetGroupMembership { channels, group } => {
            mx::set_group(project, channels, *group)?
        }
        EditCommand::AddFolder { name, tracks } => {
            mx::add_folder(project, name.as_deref(), tracks, &mut outcome)?
        }
        EditCommand::UpdateFolder { folder } => mx::update_folder(project, folder)?,
        EditCommand::RemoveFolder { id } => mx::remove_folder(project, *id)?,
        EditCommand::SetTrackFolder { ids, folder } => mx::set_track_folder(project, ids, *folder)?,
        EditCommand::SetPanLaw { law } => project.mixer.pan_law = *law,
        EditCommand::SetAutomationMode { channel, mode } => au::set_mode(project, *channel, *mode)?,
        EditCommand::AddAutomationPoints { target, points } => {
            outcome.created_points = au::add_points(project, target, points)?;
        }
        EditCommand::RemoveAutomationPoints { target, ids } => au::remove_points(project, target, ids)?,
        EditCommand::UpdateAutomationPoints { target, points } => {
            au::update_points(project, target, points)?
        }
        EditCommand::MoveAutomationPoints {
            target,
            ids,
            delta_ticks,
            delta_value,
            copy,
        } => {
            outcome.created_points =
                au::move_points(project, target, ids, *delta_ticks, *delta_value, *copy)?;
        }
        EditCommand::RemoveAutomationLane { target } => au::remove_lane(project, target)?,
        EditCommand::SimplifyAutomation { target, tolerance } => {
            au::simplify(project, target, *tolerance)?
        }
        EditCommand::WriteAutomation { writes } => {
            if writes.is_empty() || writes.len() > au::limits::MAX_WRITES {
                return Err(invalid("an automation write holds 1–4096 ranges"));
            }
            for w in writes {
                au::write(project, w)?;
            }
        }
        EditCommand::SlipRegions { ids, delta_ticks } => {
            let ids = re::phase_locked(project, ids);
            re::slip(project, &ids, *delta_ticks)?;
        }
        EditCommand::RippleDelete { ids } => {
            let ids = re::phase_locked(project, ids);
            re::ripple_delete(project, &ids)?;
        }
        EditCommand::SplitRange {
            track_ids,
            start,
            end,
        } => {
            outcome.selected_regions =
                re::split_range(project, track_ids, *start, *end, &mut outcome)?;
        }
        EditCommand::CutRange {
            track_ids,
            start,
            end,
            ripple,
        } => re::cut_range(project, track_ids, *start, *end, *ripple, &mut outcome)?,
        EditCommand::GlueRegions { ids } => {
            outcome.selected_regions = vec![re::glue(project, ids)?];
        }
        EditCommand::CreateCrossfade {
            left,
            right,
            length_ms,
        } => re::create_crossfade(project, *left, *right, *length_ms)?,
        EditCommand::SetRegionsReversed { ids, reversed } => {
            re::set_reversed(project, ids, *reversed)?
        }
        EditCommand::SetFadeCurve { ids, curve } => re::set_fade_curve(project, ids, *curve)?,
        EditCommand::SliceRegion { id, positions } => {
            re::slice(project, *id, positions, &mut outcome)?
        }
        EditCommand::NormalizeRegions { gains } => re::set_gains(project, gains)?,
        EditCommand::CompTake {
            track_id,
            folder,
            take,
            start,
            end,
        } => crate::comp::comp_take(project, *track_id, *folder, *take, *start, *end)?,
        EditCommand::SetCompCrossfade {
            track_id,
            folder,
            crossfade_ms,
        } => crate::comp::set_crossfade(project, *track_id, *folder, *crossfade_ms)?,
        EditCommand::FlattenComp { track_id, folder } => {
            crate::comp::flatten(project, *track_id, *folder, &mut outcome)?
        }
        EditCommand::UnflattenComp { track_id, folder } => {
            crate::comp::unflatten(project, *track_id, *folder)?
        }
        EditCommand::FreezeTrack { id, media, start } => {
            crate::comp::freeze(project, *id, media, *start)?
        }
        EditCommand::UnfreezeTrack { id } => crate::comp::unfreeze(project, *id)?,
        EditCommand::BounceInPlace {
            asset,
            start,
            source_track,
            regions,
            mute_sources,
        } => {
            re::bounce_in_place(
                project,
                asset,
                *start,
                *source_track,
                regions,
                *mute_sources,
                &mut outcome,
            )?;
            extend_length_to_content(project);
        }
        EditCommand::RelinkMediaFiles { items } => re::relink_all(project, items)?,
        EditCommand::RemoveUnusedMedia => re::remove_unused_media(project)?,
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
                id: 1,
                ..MidiNote::new(60, 100, 0, 480)
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
