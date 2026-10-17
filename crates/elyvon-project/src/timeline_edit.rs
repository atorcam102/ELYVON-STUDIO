//! Region, marker and media-pool edits that need the tempo map.
//!
//! Audio edits are non-destructive: the source file is never touched. A region
//! plays source frame `offset + elapsed` (wrapping around the whole file when
//! `looped`), so trimming the start advances `offset` and splitting computes
//! the second half's offset from the time elapsed at the split point.

use elyvon_core::{TempoMap, Ticks};

use crate::edit::{
    AudioPlacement, ClipboardRegion, EditOutcome, PlacementTrack, invalid, sort_regions,
};
use crate::error::EditError;
use crate::model::{
    MarkerId, MediaAsset, MediaId, Project, Region, RegionContent, RegionId, Track, TrackKind,
};
use crate::validate::limits;

fn tempo_map(project: &Project) -> Result<TempoMap, EditError> {
    project
        .transport
        .tempo_map()
        .map_err(|e| invalid(e.to_string()))
}

/// Source sample rate and length of a media file, falling back to the project
/// rate when the file was never measured (schema-1 migrations).
pub(crate) fn source_rate_and_frames(project: &Project, media: Option<MediaId>) -> (u32, Option<u64>) {
    match media
        .and_then(|id| project.media_asset(id))
        .and_then(|a| a.info.as_ref())
    {
        Some(info) => (info.sample_rate, Some(info.frames)),
        None => (project.sample_rate, None),
    }
}

/// Source frames elapsed between two ticks at `rate`.
pub fn frames_between(map: &TempoMap, from: Ticks, to: Ticks, rate: u32) -> u64 {
    let secs = map.seconds_at(to as f64) - map.seconds_at(from as f64);
    elyvon_core::tempo::round_frame(secs * f64::from(rate))
}

/// Tick at which `frames` of audio at `rate`, starting at `start`, end.
pub fn tick_after_frames(map: &TempoMap, start: Ticks, frames: u64, rate: u32) -> Ticks {
    let secs = map.seconds_at(start as f64) + frames as f64 / f64::from(rate);
    (map.tick_at_seconds(secs).round() as Ticks).max(start + 1)
}

fn wrap(offset: u64, frames: Option<u64>, looped: bool) -> u64 {
    match frames {
        Some(n) if looped && n > 0 => offset % n,
        _ => offset,
    }
}

pub(crate) fn add_marker(
    project: &mut Project,
    tick: Ticks,
    name: Option<&str>,
) -> Result<MarkerId, EditError> {
    if project.markers.len() >= limits::MAX_MARKERS {
        return Err(EditError::Limit(format!(
            "at most {} markers",
            limits::MAX_MARKERS
        )));
    }
    let name = match name {
        Some(n) => {
            crate::validate::validate_name("name", n)?;
            n.trim().to_owned()
        }
        None => {
            let mut i = project.markers.len() + 1;
            loop {
                let candidate = format!("Marker {i}");
                if !project.markers.iter().any(|m| m.name == candidate) {
                    break candidate;
                }
                i += 1;
            }
        }
    };
    let id = MarkerId::new();
    project
        .markers
        .push(crate::model::Marker { id, name, tick });
    project.markers.sort_by_key(|m| m.tick);
    Ok(id)
}

pub(crate) fn update_marker(
    project: &mut Project,
    id: MarkerId,
    name: &str,
    tick: Ticks,
) -> Result<(), EditError> {
    crate::validate::validate_name("name", name)?;
    let marker = project
        .markers
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or_else(|| invalid("unknown marker"))?;
    marker.name = name.trim().to_owned();
    marker.tick = tick;
    project.markers.sort_by_key(|m| m.tick);
    Ok(())
}

pub(crate) fn remove_markers(project: &mut Project, ids: &[MarkerId]) -> Result<(), EditError> {
    if let Some(missing) = ids
        .iter()
        .find(|id| !project.markers.iter().any(|m| m.id == **id))
    {
        return Err(invalid(format!("unknown marker {missing}")));
    }
    project.markers.retain(|m| !ids.contains(&m.id));
    Ok(())
}

pub(crate) fn import_audio(
    project: &mut Project,
    assets: &[MediaAsset],
    placements: &[AudioPlacement],
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    if project.media.len() + assets.len() > limits::MAX_MEDIA {
        return Err(EditError::Limit(format!(
            "at most {} audio files per project",
            limits::MAX_MEDIA
        )));
    }
    for asset in assets {
        if project.media_asset(asset.id).is_some() {
            return Err(invalid("media id already in the pool"));
        }
        if asset.info.is_none() {
            return Err(invalid("imported audio must be measured"));
        }
        project.media.push(asset.clone());
    }
    let map = tempo_map(project)?;
    let mut previous: Option<TrackId> = None;
    for placement in placements {
        let asset = project
            .media_asset(placement.media_id)
            .ok_or_else(|| invalid("placement refers to an unknown media file"))?;
        let info = asset
            .info
            .clone()
            .ok_or_else(|| invalid("cannot place a file that was never measured"))?;
        let name = asset.name.clone();
        let track_id = match &placement.track {
            PlacementTrack::Existing { id } => *id,
            PlacementTrack::SameAsPrevious => {
                previous.ok_or_else(|| invalid("no previous placement track"))?
            }
            PlacementTrack::New => {
                if project.tracks.len() >= limits::MAX_TRACKS {
                    return Err(EditError::Limit(format!(
                        "at most {} tracks",
                        limits::MAX_TRACKS
                    )));
                }
                let track = Track::new(TrackKind::Audio, name.clone(), project.next_track_color());
                let id = track.id;
                project.tracks.push(track);
                outcome.created_tracks.push(id);
                id
            }
        };
        previous = Some(track_id);
        let end = tick_after_frames(&map, placement.start, info.frames.max(1), info.sample_rate);
        if end > limits::MAX_TICKS {
            return Err(invalid("region would end out of range"));
        }
        let track = project
            .track_mut(track_id)
            .ok_or(EditError::UnknownTrack(track_id))?;
        if track.kind != TrackKind::Audio {
            return Err(EditError::Incompatible(
                "audio files can only be placed on audio tracks".into(),
            ));
        }
        if track.regions.len() >= limits::MAX_REGIONS_PER_TRACK {
            return Err(EditError::Limit("too many regions on this track".into()));
        }
        let region = Region {
            capture: None,
            midi_events: Vec::new(),
            id: RegionId::new(),
            name,
            start: placement.start,
            length: end - placement.start,
            muted: false,
            content: RegionContent::audio(Some(placement.media_id)),
            automation: Vec::new(),
        };
        outcome.created_regions.push(region.id);
        track.regions.push(region);
        sort_regions(track);
    }
    Ok(())
}

use crate::model::TrackId;

/// Source frame (at `rate`) of timeline tick `tick`, rounded on the absolute
/// position exactly like the engine, so differences of these values match
/// the engine's frame arithmetic for reversed regions.
pub fn abs_frame(map: &TempoMap, tick: Ticks, rate: u32) -> u64 {
    elyvon_core::tempo::round_frame(map.seconds_at(tick as f64) * f64::from(rate))
}

/// Splits region lanes at `rel` (relative ticks): the left keeps
/// `[0, rel]`, the right gets `[rel, ∞)` shifted to start at zero, each with
/// a boundary point holding the value at the cut.
fn split_lanes(
    lanes: &[crate::automation::AutomationLane],
    rel: Ticks,
) -> (
    Vec<crate::automation::AutomationLane>,
    Vec<crate::automation::AutomationLane>,
) {
    let mut left = Vec::with_capacity(lanes.len());
    let mut right = Vec::with_capacity(lanes.len());
    for lane in lanes {
        let mut l = crate::automation::AutomationLane {
            param: lane.param,
            points: lane.points.iter().filter(|p| p.tick <= rel).copied().collect(),
        };
        let mut r = crate::automation::AutomationLane {
            param: lane.param,
            points: lane
                .points
                .iter()
                .filter(|p| p.tick >= rel)
                .map(|p| crate::automation::AutomationPoint {
                    tick: p.tick - rel,
                    ..*p
                })
                .collect(),
        };
        if let Some(v) = lane.value_at(rel as f64) {
            let next = lane.points.iter().map(|p| p.id).max().unwrap_or(0) + 1;
            if l.points.last().is_none_or(|p| p.tick < rel) {
                l.points.push(crate::automation::AutomationPoint {
                    id: next,
                    tick: rel,
                    value: lane.value_before(rel as f64).unwrap_or(v),
                    curve: crate::automation::Curve::Linear,
                });
            }
            if r.points.first().is_none_or(|p| p.tick > 0) {
                r.points.insert(
                    0,
                    crate::automation::AutomationPoint {
                        id: next,
                        tick: 0,
                        value: v,
                        curve: lane
                            .points
                            .iter()
                            .rev()
                            .find(|p| p.tick <= rel)
                            .map_or(crate::automation::Curve::Linear, |p| p.curve),
                    },
                );
            }
        }
        left.push(l);
        right.push(r);
    }
    (left, right)
}

/// Splits one region at `at` (strictly inside it). Returns the right half.
fn split_one(project: &Project, map: &TempoMap, region: &mut Region, at: Ticks) -> Region {
    let rel = at - region.start;
    let end = region.end();
    let mut right = region.clone();
    right.id = RegionId::new();
    right.start = at;
    right.length = region.length - rel;
    region.length = rel;
    if !region.automation.is_empty() {
        let (l, r) = split_lanes(&region.automation, rel);
        region.automation = l;
        right.automation = r;
    }
    match (&mut region.content, &mut right.content) {
        (
            RegionContent::Audio {
                media,
                offset,
                fade_out,
                looped,
                reversed,
                ..
            },
            RegionContent::Audio {
                offset: right_offset,
                fade_in: right_fade_in,
                ..
            },
        ) => {
            let (rate, frames) = source_rate_and_frames(project, *media);
            if *reversed {
                // The left half plays the top of the source span.
                *right_offset = *offset;
                *offset += abs_frame(map, end, rate) - abs_frame(map, at, rate);
            } else {
                let elapsed = frames_between(map, at - rel, at, rate);
                *right_offset = wrap(offset.saturating_add(elapsed), frames, *looped);
            }
            *right_fade_in = 0;
            *fade_out = 0;
        }
        (RegionContent::Midi { notes }, RegionContent::Midi { notes: right_notes }) => {
            right_notes.retain(|n| n.start.saturating_add(n.length) > rel);
            for n in right_notes.iter_mut() {
                let end = n.start.saturating_add(n.length);
                n.start = n.start.saturating_sub(rel);
                n.length = end - rel - n.start;
            }
            notes.retain(|n| n.start < rel);
            for n in notes.iter_mut() {
                n.length = n.length.min(rel - n.start);
            }
        }
        _ => {}
    }
    if !right.midi_events.is_empty() {
        let source = right.midi_events.clone();
        region.midi_events = crate::io::crop_midi(&source, 0, rel);
        right.midi_events = crate::io::crop_midi(&source, rel, rel + right.length);
    }
    right
}

pub(crate) fn split_regions(
    project: &mut Project,
    ids: &[RegionId],
    at: Ticks,
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    let map = tempo_map(project)?;
    let mut any = false;
    for id in ids {
        let (ti, ri) = project
            .find_region(*id)
            .ok_or(EditError::UnknownRegion(*id))?;
        let region = &project.tracks[ti].regions[ri];
        if at <= region.start || at >= region.end() {
            continue;
        }
        if project.tracks[ti].regions.len() >= limits::MAX_REGIONS_PER_TRACK {
            return Err(EditError::Limit("too many regions on this track".into()));
        }
        let mut left = region.clone();
        let right = split_one(project, &map, &mut left, at);
        outcome.created_regions.push(right.id);
        let track = &mut project.tracks[ti];
        track.regions[ri] = left;
        track.regions.push(right);
        sort_regions(track);
        any = true;
    }
    if !any {
        return Err(invalid(
            "the split position is not inside a selected region",
        ));
    }
    Ok(())
}

pub(crate) fn trim_region(
    project: &mut Project,
    id: RegionId,
    start: Ticks,
    end: Ticks,
) -> Result<(), EditError> {
    let map = tempo_map(project)?;
    let (ti, ri) = project
        .find_region(id)
        .ok_or(EditError::UnknownRegion(id))?;
    let region = trimmed_copy(project, &map, &project.tracks[ti].regions[ri], start, end)?;
    let track = &mut project.tracks[ti];
    track.regions[ri] = region;
    sort_regions(track);
    Ok(())
}

/// A copy of `region` with its edges moved to `start`/`end` (clamped to the
/// source file when it does not loop). Audio keeps its timeline position;
/// MIDI notes, events and region automation are cropped or shifted.
pub fn trimmed_copy(
    project: &Project,
    map: &TempoMap,
    region: &Region,
    mut start: Ticks,
    mut end: Ticks,
) -> Result<Region, EditError> {
    if start >= end {
        return Err(invalid("a region must keep a positive length"));
    }
    if end > limits::MAX_TICKS {
        return Err(invalid("region would end out of range"));
    }
    let mut region = region.clone();
    let old_start = region.start;
    let old_end = region.end();
    match &mut region.content {
        RegionContent::Audio {
            media,
            offset,
            reversed: true,
            ..
        } => {
            let (rate, frames) = source_rate_and_frames(project, *media);
            // The end edge moves the bottom of the source span; the start
            // edge only extends or shortens the top.
            let old_e = abs_frame(map, old_end, rate);
            let new_e = abs_frame(map, end, rate);
            if new_e <= old_e {
                *offset += old_e - new_e;
            } else if new_e - old_e <= *offset {
                *offset -= new_e - old_e;
            } else {
                let secs = (old_e + *offset) as f64 / f64::from(rate);
                end = (map.tick_at_seconds(secs).floor() as Ticks).min(end);
                *offset = offset.saturating_sub(abs_frame(map, end, rate).saturating_sub(old_e));
            }
            if let Some(n) = frames {
                let room = n.saturating_sub(*offset);
                let e = abs_frame(map, end, rate);
                if e.saturating_sub(abs_frame(map, start, rate)) > room {
                    let secs = e.saturating_sub(room) as f64 / f64::from(rate);
                    start = (map.tick_at_seconds(secs).ceil() as Ticks).max(start);
                }
            }
        }
        RegionContent::Audio {
            media,
            offset,
            looped,
            ..
        } => {
            let (rate, frames) = source_rate_and_frames(project, *media);
            if start < old_start {
                let back = frames_between(map, start, old_start, rate);
                if back <= *offset {
                    *offset -= back;
                } else if let (true, Some(n)) = (*looped, frames.filter(|n| *n > 0)) {
                    let shift = (back - *offset) % n;
                    *offset = (n - shift) % n;
                } else {
                    // Clamp to the first source frame.
                    let secs = map.seconds_at(old_start as f64) - *offset as f64 / f64::from(rate);
                    start = (map.tick_at_seconds(secs).round() as Ticks).min(old_start);
                    *offset = 0;
                }
            } else if start > old_start {
                let fwd = frames_between(map, old_start, start, rate);
                *offset = wrap(offset.saturating_add(fwd), frames, *looped);
            }
            if let (false, Some(n)) = (*looped, frames) {
                let remaining = n.saturating_sub(*offset).max(1);
                end = end.min(tick_after_frames(map, start, remaining, rate));
            }
        }
        RegionContent::Midi { notes } => {
            if start > old_start {
                let d = start - old_start;
                notes.retain(|n| n.start + n.length > d);
                for n in notes.iter_mut() {
                    if n.start < d {
                        n.length -= d - n.start;
                        n.start = 0;
                    } else {
                        n.start -= d;
                    }
                }
            } else {
                let d = old_start - start;
                for n in notes.iter_mut() {
                    n.start += d;
                }
            }
        }
    }
    if start >= end {
        return Err(invalid("a region must keep a positive length"));
    }
    if !region.midi_events.is_empty() {
        if start >= old_start {
            region.midi_events =
                crate::io::crop_midi(&region.midi_events, start - old_start, end - old_start);
        } else {
            region.midi_events =
                crate::io::crop_midi(&region.midi_events, 0, end.saturating_sub(old_start));
            for event in &mut region.midi_events {
                event.tick += old_start - start;
            }
        }
    }
    if !region.automation.is_empty() {
        let lanes = if start > old_start {
            split_lanes(&region.automation, start - old_start).1
        } else {
            let d = old_start - start;
            let mut lanes = region.automation.clone();
            for l in &mut lanes {
                for p in &mut l.points {
                    p.tick += d;
                }
            }
            lanes
        };
        let len = end - start;
        region.automation = split_lanes(&lanes, len).0;
    }
    region.start = start;
    region.length = end - start;
    Ok(region)
}

pub(crate) fn duplicate_regions(
    project: &mut Project,
    ids: &[RegionId],
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    let mut found = Vec::with_capacity(ids.len());
    for id in ids {
        found.push(
            project
                .find_region(*id)
                .ok_or(EditError::UnknownRegion(*id))?,
        );
    }
    let first = found
        .iter()
        .map(|&(t, r)| project.tracks[t].regions[r].start)
        .min()
        .ok_or_else(|| invalid("select one or more regions"))?;
    let last = found
        .iter()
        .map(|&(t, r)| project.tracks[t].regions[r].end())
        .max()
        .unwrap_or(first);
    let span = last - first;
    let copies: Vec<(usize, Region)> = found
        .iter()
        .map(|&(t, r)| {
            let mut copy = project.tracks[t].regions[r].clone();
            copy.id = RegionId::new();
            copy.start += span;
            (t, copy)
        })
        .collect();
    for (t, copy) in copies {
        if copy.end() > limits::MAX_TICKS {
            return Err(invalid("region would move out of range"));
        }
        outcome.created_regions.push(copy.id);
        project.tracks[t].regions.push(copy);
    }
    for t in &mut project.tracks {
        if t.regions.len() > limits::MAX_REGIONS_PER_TRACK {
            return Err(EditError::Limit("too many regions on this track".into()));
        }
        sort_regions(t);
    }
    Ok(())
}

pub(crate) fn paste_regions(
    project: &mut Project,
    track_id: TrackId,
    at: Ticks,
    items: &[ClipboardRegion],
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    let base = project
        .track_index(track_id)
        .ok_or(EditError::UnknownTrack(track_id))?;
    let first = items
        .iter()
        .map(|i| i.region.start)
        .min()
        .ok_or_else(|| invalid("the clipboard is empty"))?;
    for item in items {
        let target = base + item.track_offset as usize;
        let track = project
            .tracks
            .get_mut(target)
            .ok_or_else(|| EditError::Incompatible("there is no track there".into()))?;
        if !track.kind.accepts(&item.region.content) {
            return Err(EditError::Incompatible(
                "the clipboard does not match the destination track type".into(),
            ));
        }
        let mut region = item.region.clone();
        region.id = RegionId::new();
        region.start = at
            .checked_add(region.start - first)
            .filter(|s| *s <= limits::MAX_TICKS)
            .ok_or_else(|| invalid("region would move out of range"))?;
        outcome.created_regions.push(region.id);
        track.regions.push(region);
        if track.regions.len() > limits::MAX_REGIONS_PER_TRACK {
            return Err(EditError::Limit("too many regions on this track".into()));
        }
        sort_regions(track);
    }
    Ok(())
}

pub(crate) fn set_audio_region(
    project: &mut Project,
    id: RegionId,
    gain_db: f64,
    fade_in: u64,
    fade_out: u64,
    looped: bool,
) -> Result<(), EditError> {
    if !(RegionContent::MIN_GAIN_DB..=RegionContent::MAX_GAIN_DB).contains(&gain_db) {
        return Err(invalid("region gain must be between -96 and +24 dB"));
    }
    let (ti, ri) = project
        .find_region(id)
        .ok_or(EditError::UnknownRegion(id))?;
    match &mut project.tracks[ti].regions[ri].content {
        RegionContent::Audio {
            gain_db: g,
            fade_in: fi,
            fade_out: fo,
            looped: l,
            ..
        } => {
            *g = gain_db;
            *fi = fade_in;
            *fo = fade_out;
            *l = looped;
            Ok(())
        }
        RegionContent::Midi { .. } => Err(EditError::Incompatible(
            "gain and fades apply to audio regions".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{EditCommand, apply};
    use crate::model::{MediaInfo, MidiNote};

    fn asset(rate: u32, frames: u64) -> MediaAsset {
        MediaAsset {
            id: MediaId::new(),
            name: "take.wav".into(),
            path: "/tmp/take.wav".into(),
            info: Some(MediaInfo {
                sample_rate: rate,
                channels: 2,
                frames,
                format: "wav".into(),
                size_bytes: 1,
                fingerprint: "0".into(),
            }),
        }
    }

    /// 120 BPM 4/4: one bar = 2 s.
    fn project_with_audio(rate: u32, seconds: u64) -> (Project, RegionId, MediaId) {
        let mut p = Project::new("S");
        let a = asset(rate, seconds * u64::from(rate));
        let media = a.id;
        let out = apply(
            &mut p,
            &EditCommand::ImportAudio {
                assets: vec![a],
                placements: vec![AudioPlacement {
                    media_id: media,
                    track: PlacementTrack::New,
                    start: 3840,
                }],
            },
        )
        .unwrap();
        (p, out.created_regions[0], media)
    }

    fn audio_of(p: &Project, id: RegionId) -> (Ticks, Ticks, u64, u64, u64) {
        let (t, r) = p.find_region(id).unwrap();
        let region = &p.tracks[t].regions[r];
        match region.content {
            RegionContent::Audio {
                offset,
                fade_in,
                fade_out,
                ..
            } => (region.start, region.length, offset, fade_in, fade_out),
            _ => panic!("not audio"),
        }
    }

    #[test]
    fn import_places_region_with_exact_length() {
        let (p, r, _) = project_with_audio(44_100, 4);
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.tracks[0].name, "take.wav");
        // 4 s at 120 BPM = 8 beats.
        assert_eq!(audio_of(&p, r).1, 8 * 960);
    }

    #[test]
    fn split_is_non_destructive_and_continuous() {
        let (mut p, r, _) = project_with_audio(48_000, 4);
        apply(
            &mut p,
            &EditCommand::SetAudioRegion {
                id: r,
                gain_db: -3.0,
                fade_in: 100,
                fade_out: 200,
                looped: false,
            },
        )
        .unwrap();
        let out = apply(
            &mut p,
            &EditCommand::SplitRegions {
                ids: vec![r],
                at: 3840 + 960,
            },
        )
        .unwrap();
        let right = out.created_regions[0];
        assert_eq!(audio_of(&p, r), (3840, 960, 0, 100, 0));
        // Half a second at 48 kHz later in the source.
        assert_eq!(audio_of(&p, right), (4800, 7 * 960, 24_000, 0, 200));
        // Splitting outside every region is refused.
        assert!(
            apply(
                &mut p,
                &EditCommand::SplitRegions {
                    ids: vec![r],
                    at: 0
                }
            )
            .is_err()
        );
    }

    #[test]
    fn trim_start_advances_offset_and_clamps_at_file_bounds() {
        let (mut p, r, _) = project_with_audio(48_000, 4);
        let end = 3840 + 8 * 960;
        apply(
            &mut p,
            &EditCommand::TrimRegion {
                id: r,
                start: 3840 + 480,
                end,
            },
        )
        .unwrap();
        assert_eq!(audio_of(&p, r), (4320, end - 4320, 12_000, 0, 0));
        // Extending left before the file start clamps to offset 0.
        apply(
            &mut p,
            &EditCommand::TrimRegion {
                id: r,
                start: 0,
                end,
            },
        )
        .unwrap();
        assert_eq!(audio_of(&p, r).0, 3840);
        assert_eq!(audio_of(&p, r).2, 0);
        // Extending right beyond the file end clamps (not looped).
        apply(
            &mut p,
            &EditCommand::TrimRegion {
                id: r,
                start: 3840,
                end: end + 10_000,
            },
        )
        .unwrap();
        assert_eq!(audio_of(&p, r).1, 8 * 960);
    }

    #[test]
    fn looped_regions_extend_and_wrap() {
        let (mut p, r, _) = project_with_audio(48_000, 1); // 2 beats long
        apply(
            &mut p,
            &EditCommand::SetAudioRegion {
                id: r,
                gain_db: 0.0,
                fade_in: 0,
                fade_out: 0,
                looped: true,
            },
        )
        .unwrap();
        apply(
            &mut p,
            &EditCommand::TrimRegion {
                id: r,
                start: 3840,
                end: 3840 * 3,
            },
        )
        .unwrap();
        assert_eq!(audio_of(&p, r).1, 7680);
        let out = apply(
            &mut p,
            &EditCommand::SplitRegions {
                ids: vec![r],
                at: 3840 + 960 * 3,
            },
        )
        .unwrap();
        // 1.5 s into a 1 s loop → offset 0.5 s.
        assert_eq!(audio_of(&p, out.created_regions[0]).2, 24_000);
    }

    #[test]
    fn duplicate_and_paste_get_fresh_ids() {
        let (mut p, r, _) = project_with_audio(48_000, 2);
        let out = apply(&mut p, &EditCommand::DuplicateRegions { ids: vec![r] }).unwrap();
        let copy = out.created_regions[0];
        assert_ne!(copy, r);
        assert_eq!(audio_of(&p, copy).0, 3840 + 3840);
        let (t, i) = p.find_region(r).unwrap();
        let region = p.tracks[t].regions[i].clone();
        let track = p.tracks[0].id;
        let out = apply(
            &mut p,
            &EditCommand::PasteRegions {
                track_id: track,
                at: 100_000,
                items: vec![ClipboardRegion {
                    track_offset: 0,
                    region,
                }],
            },
        )
        .unwrap();
        assert_eq!(audio_of(&p, out.created_regions[0]).0, 100_000);
        assert_eq!(p.region_count(), 3);
        // Audio cannot be pasted on a MIDI track.
        let keys = apply(
            &mut p,
            &EditCommand::AddTrack {
                kind: TrackKind::Instrument,
                name: None,
                index: None,
            },
        )
        .unwrap()
        .created_tracks[0];
        let region = p.tracks[0].regions[0].clone();
        assert!(
            apply(
                &mut p,
                &EditCommand::PasteRegions {
                    track_id: keys,
                    at: 0,
                    items: vec![ClipboardRegion {
                        track_offset: 0,
                        region
                    }],
                }
            )
            .is_err()
        );
    }

    #[test]
    fn midi_split_and_trim_partition_notes() {
        let mut p = Project::new("S");
        let k = apply(
            &mut p,
            &EditCommand::AddTrack {
                kind: TrackKind::Instrument,
                name: None,
                index: None,
            },
        )
        .unwrap()
        .created_tracks[0];
        let r = apply(
            &mut p,
            &EditCommand::CreateMidiRegion {
                track_id: k,
                start: 0,
                length: 3840,
            },
        )
        .unwrap()
        .created_regions[0];
        if let RegionContent::Midi { notes } = &mut p.tracks[0].regions[0].content {
            for (i, (start, length)) in [(0, 960), (900, 200), (2000, 100)].into_iter().enumerate()
            {
                notes.push(MidiNote {
                    id: i as u32 + 1,
                    ..MidiNote::new(60, 90, start, length)
                });
            }
        }
        let out = apply(
            &mut p,
            &EditCommand::SplitRegions {
                ids: vec![r],
                at: 1000,
            },
        )
        .unwrap();
        let notes = |p: &Project, id| {
            let (t, i) = p.find_region(id).unwrap();
            match &p.tracks[t].regions[i].content {
                RegionContent::Midi { notes } => notes
                    .iter()
                    .map(|n| (n.start, n.length))
                    .collect::<Vec<_>>(),
                _ => vec![],
            }
        };
        assert_eq!(notes(&p, r), [(0, 960), (900, 100)]);
        // Splitting retains the tail of the note crossing the cut.
        assert_eq!(notes(&p, out.created_regions[0]), [(0, 100), (1000, 100)]);
        apply(
            &mut p,
            &EditCommand::TrimRegion {
                id: r,
                start: 500,
                end: 1000,
            },
        )
        .unwrap();
        assert_eq!(notes(&p, r), [(0, 460), (400, 100)]);
    }

    #[test]
    fn markers_are_sorted_and_named() {
        let mut p = Project::new("S");
        let a = apply(
            &mut p,
            &EditCommand::AddMarker {
                tick: 7680,
                name: None,
            },
        )
        .unwrap()
        .created_markers[0];
        apply(
            &mut p,
            &EditCommand::AddMarker {
                tick: 0,
                name: Some("Intro".into()),
            },
        )
        .unwrap();
        assert_eq!(p.markers[0].name, "Intro");
        assert_eq!(p.markers[1].name, "Marker 1");
        apply(
            &mut p,
            &EditCommand::UpdateMarker {
                id: a,
                name: "Chorus".into(),
                tick: 1,
            },
        )
        .unwrap();
        assert_eq!(p.markers[1].name, "Chorus");
        apply(&mut p, &EditCommand::RemoveMarkers { ids: vec![a] }).unwrap();
        assert_eq!(p.markers.len(), 1);
        assert!(apply(&mut p, &EditCommand::RemoveMarkers { ids: vec![a] }).is_err());
    }

    #[test]
    fn tempo_and_meter_maps_are_validated() {
        let mut p = Project::new("S");
        let bad = EditCommand::SetTempoChanges {
            changes: vec![elyvon_core::TempoChange {
                tick: 0,
                bpm: elyvon_core::Tempo::DEFAULT,
            }],
        };
        assert!(apply(&mut p, &bad).is_err());
        assert!(p.transport.tempo_changes.is_empty());
        apply(
            &mut p,
            &EditCommand::SetMeterChanges {
                changes: vec![elyvon_core::MeterChange {
                    bar: 4,
                    time_signature: elyvon_core::TimeSignature::new(7, 8).unwrap(),
                }],
            },
        )
        .unwrap();
        assert_eq!(p.transport.meter_changes.len(), 1);
    }
}
