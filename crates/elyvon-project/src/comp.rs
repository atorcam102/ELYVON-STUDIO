//! Take comping and track freeze (schema 5).
//!
//! # Comps
//!
//! A cycle recording leaves one region per pass in a *take folder* (regions
//! sharing `capture.folder`). A [`TakeComp`] picks, for consecutive time
//! ranges of the folder, which take plays. The takes themselves are never
//! changed or deleted: while a comp exists they are muted and the engine
//! plays the comp's *virtual regions* ([`comp_regions`]) — trimmed copies of
//! the chosen takes with equal-power crossfades centred on every boundary.
//! [`flatten`] inserts exactly those regions into the track (so playback is
//! sample-identical) and [`unflatten`] removes them again; both are also
//! undoable.
//!
//! # Freeze
//!
//! A frozen track plays a pre-rendered file (its regions through its inserts,
//! before the fader) instead of processing them live. The file is rendered by
//! the desktop shell; [`freeze_fingerprint`] records what it was rendered
//! from. While frozen, edits that would change that material are refused; if
//! the fingerprint stops matching (a tempo change, a relinked file) the
//! engine plays the track live again and the UI reports the freeze as stale.

use std::collections::HashSet;

use elyvon_core::{TempoMap, Ticks};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edit::{EditOutcome, invalid, sort_regions};
use crate::error::{EditError, ValidationError};
use crate::model::{MediaId, Project, Region, RegionContent, RegionId, Track, TrackId, TrackKind};

pub const MAX_CROSSFADE_MS: f64 = 200.0;
pub const DEFAULT_CROSSFADE_MS: f64 = 10.0;
pub const MAX_SEGMENTS: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompSegment {
    #[ts(type = "number")]
    pub start: Ticks,
    #[ts(type = "number")]
    pub end: Ticks,
    pub take: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TakeComp {
    /// The take folder (`capture.folder` of its regions).
    pub folder: RegionId,
    /// Sorted, non-overlapping ranges; gaps are silent.
    pub segments: Vec<CompSegment>,
    /// Length of the crossfade centred on each boundary between takes.
    pub crossfade_ms: f64,
    /// The comp was consolidated into ordinary regions (`regions`); it no
    /// longer plays by itself until it is unflattened.
    #[serde(default)]
    pub flattened: bool,
    #[serde(default)]
    pub regions: Vec<RegionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrackFreeze {
    /// Rendered pre-fader file in the media pool.
    pub media: MediaId,
    /// Timeline position of the file's first frame.
    #[ts(type = "number")]
    pub start: Ticks,
    /// [`freeze_fingerprint`] of the track when it was rendered.
    pub fingerprint: String,
}

/// Regions of `folder` on `track`, ordered by take number.
pub fn folder_takes(track: &Track, folder: RegionId) -> Vec<&Region> {
    let mut takes: Vec<&Region> = track
        .regions
        .iter()
        .filter(|r| r.capture.as_ref().is_some_and(|c| c.folder == folder))
        .collect();
    takes.sort_by_key(|r| r.capture.as_ref().map_or(0, |c| c.take));
    takes
}

fn take_region(track: &Track, folder: RegionId, take: u32) -> Option<&Region> {
    track.regions.iter().find(|r| {
        r.capture
            .as_ref()
            .is_some_and(|c| c.folder == folder && c.take == take)
    })
}

/// `[first start, last end)` of a take folder.
pub fn folder_span(track: &Track, folder: RegionId) -> Option<(Ticks, Ticks)> {
    let takes = folder_takes(track, folder);
    let start = takes.iter().map(|r| r.start).min()?;
    let end = takes.iter().map(|r| r.end()).max()?;
    Some((start, end))
}

fn tempo_map(project: &Project) -> Result<TempoMap, EditError> {
    project
        .transport
        .tempo_map()
        .map_err(|e| invalid(e.to_string()))
}

fn shift_tick(map: &TempoMap, tick: Ticks, seconds: f64) -> Ticks {
    let s = map.seconds_at(tick as f64) + seconds;
    if s <= 0.0 {
        0
    } else {
        map.tick_at_seconds(s).round() as Ticks
    }
}

/// The ordinary audio regions a comp plays as (fresh ids, no capture data).
pub fn comp_regions(project: &Project, map: &TempoMap, track: &Track, comp: &TakeComp) -> Vec<Region> {
    let half = comp.crossfade_ms.clamp(0.0, MAX_CROSSFADE_MS) / 2000.0;
    let segs = &comp.segments;
    let mut out = Vec::with_capacity(segs.len());
    for (k, seg) in segs.iter().enumerate() {
        let Some(take) = take_region(track, comp.folder, seg.take) else {
            continue;
        };
        let prev = k > 0 && segs[k - 1].end == seg.start && segs[k - 1].take != seg.take;
        let next = segs.get(k + 1).is_some_and(|n| n.start == seg.end && n.take != seg.take);
        let start = if prev { shift_tick(map, seg.start, -half) } else { seg.start };
        let end = if next { shift_tick(map, seg.end, half) } else { seg.end };
        let start = start.max(take.start);
        let end = end.min(take.end());
        if start >= end {
            continue;
        }
        let Ok(mut region) = crate::timeline_edit::trimmed_copy(project, map, take, start, end) else {
            continue;
        };
        if let RegionContent::Audio {
            media,
            fade_in,
            fade_out,
            ..
        } = &mut region.content
        {
            let (rate, _) = crate::timeline_edit::source_rate_and_frames(project, *media);
            let take_fades = match &take.content {
                RegionContent::Audio {
                    fade_in, fade_out, ..
                } => (*fade_in, *fade_out),
                RegionContent::Midi { .. } => (0, 0),
            };
            *fade_in = if prev {
                let fade_end = shift_tick(map, seg.start, half).min(end);
                crate::timeline_edit::frames_between(map, start, fade_end, rate)
            } else if start == take.start {
                take_fades.0
            } else {
                0
            };
            *fade_out = if !next && end == take.end() { take_fades.1 } else { 0 };
        }
        region.id = RegionId::new();
        region.capture = None;
        region.muted = false;
        region.automation.clear();
        region.name = format!("Comp {}", out.len() + 1);
        out.push(region);
    }
    out
}

fn comp_mut(track: &mut Track, folder: RegionId) -> Option<&mut TakeComp> {
    track.comps.iter_mut().find(|c| c.folder == folder)
}

fn check_folder(track: &Track, folder: RegionId) -> Result<(Ticks, Ticks), EditError> {
    if track.kind != TrackKind::Audio {
        return Err(EditError::Incompatible("comping works on audio take folders".into()));
    }
    folder_span(track, folder).ok_or_else(|| invalid("the take folder does not exist"))
}

fn mute_takes(track: &mut Track, folder: RegionId, active: Option<u32>) {
    for r in &mut track.regions {
        if let Some(c) = &r.capture
            && c.folder == folder
        {
            r.muted = Some(c.take) != active;
        }
    }
}

/// Assigns `[start, end)` of the folder to `take` (creating the comp from
/// the active take when there is none yet).
pub(crate) fn comp_take(
    project: &mut Project,
    track_id: TrackId,
    folder: RegionId,
    take: u32,
    start: Ticks,
    end: Ticks,
) -> Result<(), EditError> {
    let track = project.track_mut(track_id).ok_or(EditError::UnknownTrack(track_id))?;
    let (fs, fe) = check_folder(track, folder)?;
    if take_region(track, folder, take).is_none() {
        return Err(invalid("take does not exist"));
    }
    let (start, end) = (start.max(fs), end.min(fe));
    if start >= end {
        return Err(invalid("the comp range is outside the take folder"));
    }
    if comp_mut(track, folder).is_none() {
        let takes = folder_takes(track, folder);
        let active = takes
            .iter()
            .rev()
            .find(|r| !r.muted)
            .or(takes.last())
            .and_then(|r| r.capture.as_ref().map(|c| c.take))
            .unwrap_or(take);
        track.comps.push(TakeComp {
            folder,
            segments: vec![CompSegment {
                start: fs,
                end: fe,
                take: active,
            }],
            crossfade_ms: DEFAULT_CROSSFADE_MS,
            flattened: false,
            regions: Vec::new(),
        });
    }
    let comp = comp_mut(track, folder).expect("created above");
    if comp.flattened {
        return Err(EditError::Incompatible(
            "the comp is flattened; unflatten it to change the takes".into(),
        ));
    }
    let mut segs = Vec::with_capacity(comp.segments.len() + 2);
    for s in &comp.segments {
        if s.end <= start || s.start >= end {
            segs.push(*s);
            continue;
        }
        if s.start < start {
            segs.push(CompSegment { end: start, ..*s });
        }
        if s.end > end {
            segs.push(CompSegment { start: end, ..*s });
        }
    }
    segs.push(CompSegment { start, end, take });
    segs.sort_by_key(|s| s.start);
    // Merge neighbours that play the same take.
    let mut merged: Vec<CompSegment> = Vec::with_capacity(segs.len());
    for s in segs {
        match merged.last_mut() {
            Some(m) if m.end == s.start && m.take == s.take => m.end = s.end,
            _ => merged.push(s),
        }
    }
    if merged.len() > MAX_SEGMENTS {
        return Err(EditError::Limit(format!("at most {MAX_SEGMENTS} comp segments")));
    }
    comp.segments = merged;
    mute_takes(track, folder, None);
    Ok(())
}

/// Plays one whole take (removes the comp).
pub(crate) fn select_take(
    project: &mut Project,
    track_id: TrackId,
    folder: RegionId,
    take: u32,
) -> Result<(), EditError> {
    let track = project.track_mut(track_id).ok_or(EditError::UnknownTrack(track_id))?;
    if take_region(track, folder, take).is_none() {
        return Err(invalid("take does not exist"));
    }
    if comp_mut(track, folder).is_some_and(|c| c.flattened) {
        return Err(EditError::Incompatible(
            "the comp is flattened; unflatten it before choosing another take".into(),
        ));
    }
    track.comps.retain(|c| c.folder != folder);
    mute_takes(track, folder, Some(take));
    Ok(())
}

pub(crate) fn set_crossfade(
    project: &mut Project,
    track_id: TrackId,
    folder: RegionId,
    ms: f64,
) -> Result<(), EditError> {
    if !ms.is_finite() || !(0.0..=MAX_CROSSFADE_MS).contains(&ms) {
        return Err(invalid("comp crossfades are 0–200 ms"));
    }
    let track = project.track_mut(track_id).ok_or(EditError::UnknownTrack(track_id))?;
    let comp = comp_mut(track, folder).ok_or_else(|| invalid("the take folder has no comp"))?;
    if comp.flattened {
        return Err(EditError::Incompatible("unflatten the comp to change its crossfades".into()));
    }
    comp.crossfade_ms = ms;
    Ok(())
}

pub(crate) fn flatten(
    project: &mut Project,
    track_id: TrackId,
    folder: RegionId,
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    let map = tempo_map(project)?;
    let ti = project.track_index(track_id).ok_or(EditError::UnknownTrack(track_id))?;
    let track = &project.tracks[ti];
    let comp = track
        .comps
        .iter()
        .find(|c| c.folder == folder)
        .ok_or_else(|| invalid("the take folder has no comp"))?;
    if comp.flattened {
        return Err(invalid("the comp is already flattened"));
    }
    let regions = comp_regions(project, &map, track, comp);
    if regions.is_empty() {
        return Err(invalid("the comp has nothing to consolidate"));
    }
    let ids: Vec<RegionId> = regions.iter().map(|r| r.id).collect();
    let track = &mut project.tracks[ti];
    if track.regions.len() + regions.len() > crate::validate::limits::MAX_REGIONS_PER_TRACK {
        return Err(EditError::Limit("too many regions on this track".into()));
    }
    track.regions.extend(regions);
    sort_regions(track);
    let comp = comp_mut(track, folder).expect("found above");
    comp.flattened = true;
    comp.regions = ids.clone();
    outcome.created_regions.extend(ids);
    Ok(())
}

pub(crate) fn unflatten(project: &mut Project, track_id: TrackId, folder: RegionId) -> Result<(), EditError> {
    let track = project.track_mut(track_id).ok_or(EditError::UnknownTrack(track_id))?;
    let comp = comp_mut(track, folder).ok_or_else(|| invalid("the take folder has no comp"))?;
    if !comp.flattened {
        return Err(invalid("the comp is not flattened"));
    }
    let ids = std::mem::take(&mut comp.regions);
    comp.flattened = false;
    track.regions.retain(|r| !ids.contains(&r.id));
    Ok(())
}

/// Keeps comps consistent after any edit: drops comps of folders without
/// takes, segments of deleted takes and references to deleted regions.
pub(crate) fn prune(project: &mut Project) {
    for track in &mut project.tracks {
        if track.comps.is_empty() {
            continue;
        }
        let existing: HashSet<RegionId> = track.regions.iter().map(|r| r.id).collect();
        let takes: HashSet<(RegionId, u32)> = track
            .regions
            .iter()
            .filter_map(|r| r.capture.as_ref().map(|c| (c.folder, c.take)))
            .collect();
        track.comps.retain(|c| takes.iter().any(|(f, _)| *f == c.folder));
        for comp in &mut track.comps {
            let folder = comp.folder;
            comp.segments.retain(|s| takes.contains(&(folder, s.take)));
            comp.regions.retain(|id| existing.contains(id));
        }
    }
}

pub(crate) fn validate_track(path: &str, track: &Track) -> Result<(), ValidationError> {
    let mut folders = HashSet::new();
    for (i, comp) in track.comps.iter().enumerate() {
        let cp = format!("{path}.comps[{i}]");
        if !folders.insert(comp.folder) {
            return Err(ValidationError::new(cp, "duplicate comp for a take folder"));
        }
        if !comp.crossfade_ms.is_finite() || !(0.0..=MAX_CROSSFADE_MS).contains(&comp.crossfade_ms) {
            return Err(ValidationError::new(cp, "crossfade must be 0–200 ms"));
        }
        if comp.segments.len() > MAX_SEGMENTS {
            return Err(ValidationError::new(cp, "too many segments"));
        }
        let mut prev_end = 0;
        for s in &comp.segments {
            if s.start >= s.end || s.start < prev_end {
                return Err(ValidationError::new(cp, "segments must be ordered and must not overlap"));
            }
            prev_end = s.end;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Freeze

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Identifies everything a frozen file was rendered from: regions, comps,
/// inserts and their automation, the tempo map, the sample rate and the
/// measured media.
pub fn freeze_fingerprint(project: &Project, track: &Track) -> String {
    let insert_lanes: Vec<_> = track
        .automation
        .lanes
        .iter()
        .filter(|l| matches!(l.param, crate::automation::AutomationParam::Insert { .. }))
        .collect();
    let media: Vec<_> = track
        .regions
        .iter()
        .filter_map(|r| match &r.content {
            RegionContent::Audio { media: Some(id), .. } => project.media_asset(*id).map(|a| &a.info),
            _ => None,
        })
        .collect();
    let t = &project.transport;
    let value = serde_json::json!({
        "regions": track.regions,
        "comps": track.comps,
        "inserts": track.strip.inserts,
        "format": track.strip.format,
        "insertLanes": insert_lanes,
        "tempo": t.tempo,
        "tempoChanges": t.tempo_changes,
        "timeSignature": t.time_signature,
        "meterChanges": t.meter_changes,
        "sampleRate": project.sample_rate,
        "media": media,
    });
    format!("{:016x}", fnv(value.to_string().as_bytes()))
}

/// Whether the track's freeze still matches what it was rendered from.
pub fn freeze_is_current(project: &Project, track: &Track) -> bool {
    track
        .freeze
        .as_ref()
        .is_some_and(|f| f.fingerprint == freeze_fingerprint(project, track) && project.media_asset(f.media).is_some())
}

pub(crate) fn freeze(
    project: &mut Project,
    id: TrackId,
    asset: &crate::model::MediaAsset,
    start: Ticks,
) -> Result<(), EditError> {
    let track = project.track(id).ok_or(EditError::UnknownTrack(id))?;
    if track.kind == TrackKind::Midi {
        return Err(EditError::Incompatible("external MIDI tracks have no audio to freeze".into()));
    }
    if asset.info.is_none() {
        return Err(invalid("a freeze file must be measured"));
    }
    let fingerprint = freeze_fingerprint(project, track);
    if project.media_asset(asset.id).is_none() {
        if project.media.len() >= crate::validate::limits::MAX_MEDIA {
            return Err(EditError::Limit("too many audio files".into()));
        }
        project.media.push(asset.clone());
    }
    let track = project.track_mut(id).ok_or(EditError::UnknownTrack(id))?;
    track.freeze = Some(TrackFreeze {
        media: asset.id,
        start,
        fingerprint,
    });
    Ok(())
}

pub(crate) fn unfreeze(project: &mut Project, id: TrackId) -> Result<(), EditError> {
    let track = project.track_mut(id).ok_or(EditError::UnknownTrack(id))?;
    if track.freeze.take().is_none() {
        return Err(invalid("the track is not frozen"));
    }
    Ok(())
}

/// Refuses edits that change the material of a frozen track.
pub(crate) fn check_frozen(before: &Project, after: &Project) -> Result<(), EditError> {
    for t in &after.tracks {
        let (Some(f), Some(old)) = (&t.freeze, before.track(t.id)) else {
            continue;
        };
        if old.freeze.as_ref() != Some(f) {
            continue; // just frozen
        }
        let insert_lanes = |t: &Track| -> Vec<crate::automation::AutomationLane> {
            t.automation
                .lanes
                .iter()
                .filter(|l| matches!(l.param, crate::automation::AutomationParam::Insert { .. }))
                .cloned()
                .collect()
        };
        if old.regions != t.regions
            || old.comps != t.comps
            || old.strip.inserts != t.strip.inserts
            || old.strip.format != t.strip.format
            || insert_lanes(old) != insert_lanes(t)
        {
            return Err(EditError::Incompatible(format!(
                "“{}” is frozen; unfreeze it to edit its regions or inserts",
                t.name
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_freeze(path: &str, track: &Track, media: &HashSet<MediaId>) -> Result<(), ValidationError> {
    if let Some(f) = &track.freeze {
        if !media.contains(&f.media) {
            return Err(ValidationError::new(
                format!("{path}.freeze.media"),
                "refers to a file that is not in the media pool",
            ));
        }
        if f.fingerprint.len() > 32 {
            return Err(ValidationError::new(format!("{path}.freeze"), "invalid fingerprint"));
        }
    }
    Ok(())
}
