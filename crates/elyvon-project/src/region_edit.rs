//! Non-destructive region editing (Phase 11): slip, ripple delete, range
//! (marquee) split and cut, glue, editable crossfades, reverse, fade shapes,
//! slicing, normalisation gains, bounce placement, media relinking and
//! phase-locked group edits.
//!
//! Every operation only changes how regions reference their media (offsets,
//! lengths, fades, gain, direction). Source files are never written; files
//! created by bounce and freeze are new files added to the pool.

use std::collections::HashSet;

use elyvon_core::{TempoMap, Ticks};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edit::{EditOutcome, invalid, sort_regions};
use crate::error::EditError;
use crate::model::{
    FadeCurve, MediaAsset, MediaId, MediaInfo, Project, Region, RegionContent, RegionId, Track,
    TrackId, TrackKind,
};
use crate::timeline_edit::{abs_frame, frames_between, source_rate_and_frames, trimmed_copy};
use crate::validate::limits;

pub const MAX_SLICES: usize = 1024;
pub const MAX_CROSSFADE_MS: f64 = 2000.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RegionGain {
    pub id: RegionId,
    pub gain_db: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaRelink {
    pub id: MediaId,
    pub path: String,
    pub info: Option<MediaInfo>,
}

fn tempo_map(project: &Project) -> Result<TempoMap, EditError> {
    project
        .transport
        .tempo_map()
        .map_err(|e| invalid(e.to_string()))
}

fn locate(project: &Project, id: RegionId) -> Result<(usize, usize), EditError> {
    project.find_region(id).ok_or(EditError::UnknownRegion(id))
}

/// Seconds between `start` and `start + delta` (negative `delta` allowed,
/// extrapolated with the first tempo before zero).
fn seconds_for(map: &TempoMap, start: Ticks, delta: i64) -> f64 {
    let s0 = map.seconds_at(start as f64);
    if delta >= 0 || start >= delta.unsigned_abs() {
        map.seconds_at((start as i64 + delta) as f64) - s0
    } else {
        let to_zero = -s0;
        let beyond = (delta.unsigned_abs() - start) as f64 * 60.0 / (map.initial_bpm() * 960.0);
        to_zero - beyond
    }
}

// ---------------------------------------------------------------------------
// Phase-locked group edits

/// `ids` plus, for tracks in a group with linked edits, the regions of the
/// other members at exactly the same position and length.
pub fn phase_locked(project: &Project, ids: &[RegionId]) -> Vec<RegionId> {
    let mut out: Vec<RegionId> = ids.to_vec();
    let mut seen: HashSet<RegionId> = ids.iter().copied().collect();
    for id in ids {
        let Some((ti, ri)) = project.find_region(*id) else {
            continue;
        };
        let track = &project.tracks[ti];
        let Some(group) = track.strip.group.and_then(|g| project.mixer.group(g)) else {
            continue;
        };
        if !group.link_edit {
            continue;
        }
        let region = &track.regions[ri];
        for other in project
            .tracks
            .iter()
            .filter(|t| t.id != track.id && t.strip.group == Some(group.id))
        {
            for r in other
                .regions
                .iter()
                .filter(|r| r.start == region.start && r.length == region.length)
            {
                if seen.insert(r.id) {
                    out.push(r.id);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Slip

/// Moves the content inside regions by `delta_ticks` (positive = later)
/// without moving the regions. Audio stops at the source file's edges
/// unless it loops; MIDI notes move and are cropped to the region.
pub(crate) fn slip(project: &mut Project, ids: &[RegionId], delta_ticks: i64) -> Result<(), EditError> {
    if ids.is_empty() {
        return Err(invalid("select the regions to slip"));
    }
    let map = tempo_map(project)?;
    for id in ids {
        let (ti, ri) = locate(project, *id)?;
        let region = &project.tracks[ti].regions[ri];
        let start = region.start;
        let length = region.length;
        let mut content = region.content.clone();
        match &mut content {
            RegionContent::Audio {
                media,
                offset,
                looped,
                reversed,
                ..
            } => {
                let (rate, frames) = source_rate_and_frames(project, *media);
                let secs = seconds_for(&map, start, delta_ticks);
                let shift = (secs.abs() * f64::from(rate)).round() as u64;
                let heard = abs_frame(&map, start + length, rate) - abs_frame(&map, start, rate);
                let later = (delta_ticks > 0) != *reversed;
                if *looped {
                    if let Some(n) = frames.filter(|n| *n > 0) {
                        let s = shift % n;
                        *offset = if later { (*offset + n - s) % n } else { (*offset + s) % n };
                    }
                } else if later {
                    *offset = offset.saturating_sub(shift);
                } else {
                    let max = frames.map_or(u64::MAX, |n| n.saturating_sub(heard));
                    *offset = offset.saturating_add(shift).min(max.max(*offset));
                }
            }
            RegionContent::Midi { notes } => {
                let len = length as i64;
                notes.retain_mut(|n| {
                    let s = n.start as i64 + delta_ticks;
                    let e = s + n.length as i64;
                    if e <= 0 || s >= len {
                        return false;
                    }
                    let s2 = s.max(0);
                    n.start = s2 as u64;
                    n.length = (e.min(len) - s2) as u64;
                    true
                });
            }
        }
        if let RegionContent::Midi { .. } = content {
            let r = &mut project.tracks[ti].regions[ri];
            if !r.midi_events.is_empty() {
                let shifted: Vec<_> = r
                    .midi_events
                    .iter()
                    .filter_map(|e| {
                        let t = e.tick as i64 + delta_ticks;
                        (0..=length as i64).contains(&t).then(|| crate::io::StoredMidiEvent {
                            tick: t as u64,
                            bytes: e.bytes.clone(),
                        })
                    })
                    .collect();
                r.midi_events = shifted;
            }
        }
        project.tracks[ti].regions[ri].content = content;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Ripple delete and ranges

/// Merged `[start, end)` intervals.
fn merge(mut v: Vec<(Ticks, Ticks)>) -> Vec<(Ticks, Ticks)> {
    v.sort_unstable();
    let mut out: Vec<(Ticks, Ticks)> = Vec::with_capacity(v.len());
    for (s, e) in v {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// How far a position after removed `cuts` moves left.
fn ripple_shift(cuts: &[(Ticks, Ticks)], t: Ticks) -> Ticks {
    let mut shift = 0;
    for &(s, e) in cuts {
        if t >= e {
            shift += e - s;
        } else if t > s {
            shift += t - s;
        }
    }
    shift
}

/// Closes the gaps `cuts` on `track`: later regions and track automation
/// move left; automation inside the cuts is removed.
fn close_gaps(track: &mut Track, cuts: &[(Ticks, Ticks)]) {
    if cuts.is_empty() {
        return;
    }
    for r in &mut track.regions {
        r.start -= ripple_shift(cuts, r.start);
    }
    for lane in &mut track.automation.lanes {
        lane.points.retain(|p| !cuts.iter().any(|&(s, e)| p.tick > s && p.tick < e));
        for p in &mut lane.points {
            p.tick -= ripple_shift(cuts, p.tick);
        }
        lane.points.sort_by_key(|p| p.tick);
    }
    sort_regions(track);
}

pub(crate) fn ripple_delete(project: &mut Project, ids: &[RegionId]) -> Result<(), EditError> {
    if ids.is_empty() {
        return Err(invalid("select the regions to delete"));
    }
    for id in ids {
        locate(project, *id)?;
    }
    for track in &mut project.tracks {
        let cuts = merge(
            track
                .regions
                .iter()
                .filter(|r| ids.contains(&r.id))
                .map(|r| (r.start, r.end()))
                .collect(),
        );
        if cuts.is_empty() {
            continue;
        }
        track.regions.retain(|r| !ids.contains(&r.id));
        // A kept region starting inside a cut moves to the cut's start.
        close_gaps(track, &cuts);
    }
    Ok(())
}

/// Splits the regions of `track_ids` at `start` and `end` and returns the
/// regions inside the range (the marquee selection).
pub(crate) fn split_range(
    project: &mut Project,
    track_ids: &[TrackId],
    start: Ticks,
    end: Ticks,
    outcome: &mut EditOutcome,
) -> Result<Vec<RegionId>, EditError> {
    if start >= end {
        return Err(invalid("the range must have a positive length"));
    }
    if track_ids.is_empty() {
        return Err(invalid("choose the tracks of the range"));
    }
    for id in track_ids {
        project.track(*id).ok_or(EditError::UnknownTrack(*id))?;
    }
    for at in [start, end] {
        let ids: Vec<RegionId> = project
            .tracks
            .iter()
            .filter(|t| track_ids.contains(&t.id))
            .flat_map(|t| t.regions.iter())
            .filter(|r| r.start < at && r.end() > at)
            .map(|r| r.id)
            .collect();
        if !ids.is_empty() {
            crate::timeline_edit::split_regions(project, &ids, at, outcome)?;
        }
    }
    Ok(project
        .tracks
        .iter()
        .filter(|t| track_ids.contains(&t.id))
        .flat_map(|t| t.regions.iter())
        .filter(|r| r.start >= start && r.end() <= end)
        .map(|r| r.id)
        .collect())
}

/// Removes everything inside the range on `track_ids`; with `ripple` the
/// material after it moves left to close the gap.
pub(crate) fn cut_range(
    project: &mut Project,
    track_ids: &[TrackId],
    start: Ticks,
    end: Ticks,
    ripple: bool,
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    let inside = split_range(project, track_ids, start, end, outcome)?;
    outcome.created_regions.retain(|id| !inside.contains(id));
    for track in project.tracks.iter_mut().filter(|t| track_ids.contains(&t.id)) {
        track.regions.retain(|r| !inside.contains(&r.id));
        if ripple {
            close_gaps(track, &[(start, end)]);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Glue

/// Joins regions of one track into one. Audio must continue the same
/// source without gaps (otherwise bounce them); MIDI regions merge their
/// notes, events and region automation.
pub(crate) fn glue(project: &mut Project, ids: &[RegionId]) -> Result<RegionId, EditError> {
    if ids.len() < 2 {
        return Err(invalid("select two or more regions to glue"));
    }
    let map = tempo_map(project)?;
    let ti = locate(project, ids[0])?.0;
    let mut regions: Vec<Region> = Vec::with_capacity(ids.len());
    for id in ids {
        let (t, r) = locate(project, *id)?;
        if t != ti {
            return Err(EditError::Incompatible("glue joins regions of one track".into()));
        }
        regions.push(project.tracks[t].regions[r].clone());
    }
    regions.sort_by_key(|r| r.start);
    let mut out = regions[0].clone();
    for w in regions.windows(2) {
        if w[1].start < w[0].end() {
            return Err(EditError::Incompatible("overlapping regions cannot be glued".into()));
        }
    }
    let last = regions.last().expect("two or more");
    match &mut out.content {
        RegionContent::Audio {
            media,
            looped,
            reversed,
            fade_out,
            ..
        } => {
            if *looped || *reversed {
                return Err(EditError::Incompatible(
                    "looped or reversed regions cannot be glued; bounce them instead".into(),
                ));
            }
            let (rate, _) = source_rate_and_frames(project, *media);
            for w in regions.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                let (
                    RegionContent::Audio {
                        media: ma,
                        offset: oa,
                        gain_db: ga,
                        ..
                    },
                    RegionContent::Audio {
                        media: mb,
                        offset: ob,
                        gain_db: gb,
                        looped: lb,
                        reversed: rb,
                        ..
                    },
                ) = (&a.content, &b.content)
                else {
                    return Err(EditError::Incompatible("audio and MIDI cannot be glued".into()));
                };
                let expected = oa + frames_between(&map, a.start, a.end(), rate);
                if ma != mb
                    || *lb
                    || *rb
                    || (ga - gb).abs() > 1e-9
                    || b.start != a.end()
                    || expected.abs_diff(*ob) > 1
                {
                    return Err(EditError::Incompatible(
                        "these regions do not continue the same audio; use Bounce in Place to glue them into a new file"
                            .into(),
                    ));
                }
            }
            if let RegionContent::Audio { fade_out: f, .. } = &last.content {
                *fade_out = *f;
            }
        }
        RegionContent::Midi { notes } => {
            let mut next_id = notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
            for r in &regions[1..] {
                let RegionContent::Midi { notes: more } = &r.content else {
                    return Err(EditError::Incompatible("audio and MIDI cannot be glued".into()));
                };
                let d = r.start - out.start;
                for n in more {
                    notes.push(crate::model::MidiNote {
                        id: next_id,
                        start: n.start + d,
                        ..*n
                    });
                    next_id += 1;
                }
                for e in &r.midi_events {
                    out.midi_events.push(crate::io::StoredMidiEvent {
                        tick: e.tick + d,
                        bytes: e.bytes.clone(),
                    });
                }
            }
            notes.sort_by_key(|n| (n.start, n.pitch));
            out.midi_events.sort_by_key(|e| e.tick);
        }
    }
    // Region lanes continue across the joined regions.
    for r in &regions[1..] {
        let d = r.start - out.start;
        for lane in &r.automation {
            let target = match out.automation.iter_mut().find(|l| l.param == lane.param) {
                Some(l) => l,
                None => {
                    out.automation.push(crate::automation::AutomationLane {
                        param: lane.param,
                        points: Vec::new(),
                    });
                    out.automation.last_mut().expect("pushed")
                }
            };
            let mut id = target.points.iter().map(|p| p.id).max().unwrap_or(0) + 1;
            for p in &lane.points {
                target.points.push(crate::automation::AutomationPoint {
                    id,
                    tick: p.tick + d,
                    ..*p
                });
                id += 1;
            }
            target.points.sort_by_key(|p| p.tick);
        }
    }
    out.length = last.end() - out.start;
    let id = out.id;
    let track = &mut project.tracks[ti];
    track.regions.retain(|r| !ids.contains(&r.id));
    track.regions.push(out);
    sort_regions(track);
    Ok(id)
}

// ---------------------------------------------------------------------------
// Crossfades, reverse, fade shapes

/// Overlaps two touching audio regions by up to `length_ms`, centred on
/// their boundary (as far as each file has audio beyond the edge), and
/// fades the right region in over the overlap. The engine plays the covered
/// part of the left region with the complementary fade-out.
pub(crate) fn create_crossfade(
    project: &mut Project,
    left: RegionId,
    right: RegionId,
    length_ms: f64,
) -> Result<(), EditError> {
    if !length_ms.is_finite() || length_ms <= 0.0 || length_ms > MAX_CROSSFADE_MS {
        return Err(invalid("crossfades are 1–2000 ms"));
    }
    let map = tempo_map(project)?;
    let (lt, li) = locate(project, left)?;
    let (rt, ri) = locate(project, right)?;
    if lt != rt {
        return Err(EditError::Incompatible("crossfades join regions of one track".into()));
    }
    let a = project.tracks[lt].regions[li].clone();
    let b = project.tracks[rt].regions[ri].clone();
    let (a, b) = if a.start <= b.start { (a, b) } else { (b, a) };
    let (
        RegionContent::Audio {
            media: ma,
            offset: oa,
            looped: la,
            reversed: ra,
            ..
        },
        RegionContent::Audio {
            media: mb,
            offset: ob,
            looped: lb,
            reversed: rb,
            ..
        },
    ) = (&a.content, &b.content)
    else {
        return Err(EditError::Incompatible("crossfades join audio regions".into()));
    };
    let (rate_a, frames_a) = source_rate_and_frames(project, *ma);
    let (rate_b, frames_b) = source_rate_and_frames(project, *mb);
    let half = length_ms / 2000.0;
    let (new_a_end, new_b_start) = if b.start < a.end() {
        // Already overlapping: fade over the existing overlap.
        (a.end(), b.start)
    } else if b.start == a.end() {
        let boundary = a.end();
        let heard_a = abs_frame(&map, a.end(), rate_a) - abs_frame(&map, a.start, rate_a);
        // Audio available after the left region's end and before the right's start.
        let after_a = if *la {
            f64::INFINITY
        } else if *ra {
            *oa as f64 / f64::from(rate_a)
        } else {
            frames_a.map_or(f64::INFINITY, |n| n.saturating_sub(oa + heard_a) as f64 / f64::from(rate_a))
        };
        let heard_b = abs_frame(&map, b.end(), rate_b) - abs_frame(&map, b.start, rate_b);
        let before_b = if *lb {
            f64::INFINITY
        } else if *rb {
            frames_b.map_or(f64::INFINITY, |n| n.saturating_sub(ob + heard_b) as f64 / f64::from(rate_b))
        } else {
            *ob as f64 / f64::from(rate_b)
        };
        let post = half.min(after_a);
        let pre = half.min(before_b);
        if post + pre <= 0.0 {
            return Err(EditError::Incompatible(
                "neither file has audio beyond the cut to crossfade into".into(),
            ));
        }
        let s = map.seconds_at(boundary as f64);
        let end = (map.tick_at_seconds(s + post).floor() as Ticks).min(b.end().saturating_sub(1));
        let start = (map.tick_at_seconds(s - pre).ceil() as Ticks).max(a.start + 1);
        (end.max(boundary), start.min(boundary))
    } else {
        return Err(EditError::Incompatible(
            "crossfades join touching or overlapping regions".into(),
        ));
    };
    let new_a = trimmed_copy(project, &map, &a, a.start, new_a_end)?;
    let mut new_b = trimmed_copy(project, &map, &b, new_b_start, b.end())?;
    let overlap_end = new_a.end().max(new_b.start);
    if let RegionContent::Audio { fade_in, .. } = &mut new_b.content {
        *fade_in = frames_between(&map, new_b.start, overlap_end, rate_b);
    }
    let mut new_a = new_a;
    if let RegionContent::Audio { fade_out, .. } = &mut new_a.content {
        // The covering region's fade-in provides the left half.
        *fade_out = 0;
    }
    let track = &mut project.tracks[lt];
    for r in &mut track.regions {
        if r.id == new_a.id {
            *r = new_a.clone();
        } else if r.id == new_b.id {
            *r = new_b.clone();
        }
    }
    sort_regions(track);
    Ok(())
}

fn audio_mut(project: &mut Project, id: RegionId) -> Result<&mut RegionContent, EditError> {
    let (t, r) = locate(project, id)?;
    let c = &mut project.tracks[t].regions[r].content;
    match c {
        RegionContent::Audio { .. } => Ok(c),
        RegionContent::Midi { .. } => Err(EditError::Incompatible("this applies to audio regions".into())),
    }
}

pub(crate) fn set_reversed(project: &mut Project, ids: &[RegionId], on: bool) -> Result<(), EditError> {
    if ids.is_empty() {
        return Err(invalid("select the audio regions to reverse"));
    }
    for id in ids {
        if let RegionContent::Audio { looped, reversed, .. } = audio_mut(project, *id)? {
            if *looped && on {
                return Err(EditError::Incompatible(
                    "looped regions cannot be reversed; turn looping off first".into(),
                ));
            }
            *reversed = on;
        }
    }
    Ok(())
}

pub(crate) fn set_fade_curve(project: &mut Project, ids: &[RegionId], curve: FadeCurve) -> Result<(), EditError> {
    if ids.is_empty() {
        return Err(invalid("select the audio regions"));
    }
    for id in ids {
        if let RegionContent::Audio { fade_curve, .. } = audio_mut(project, *id)? {
            *fade_curve = curve;
        }
    }
    Ok(())
}

/// Applies measured normalisation gains (non-destructive region gain).
pub(crate) fn set_gains(project: &mut Project, gains: &[RegionGain]) -> Result<(), EditError> {
    if gains.is_empty() {
        return Err(invalid("nothing to normalise"));
    }
    for g in gains {
        if !g.gain_db.is_finite() || !(RegionContent::MIN_GAIN_DB..=RegionContent::MAX_GAIN_DB).contains(&g.gain_db) {
            return Err(invalid("region gain must be between -96 and +24 dB"));
        }
        if let RegionContent::Audio { gain_db, .. } = audio_mut(project, g.id)? {
            *gain_db = g.gain_db;
        }
    }
    Ok(())
}

/// Splits a region at every position strictly inside it (slices).
pub(crate) fn slice(
    project: &mut Project,
    id: RegionId,
    positions: &[Ticks],
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    let (t, r) = locate(project, id)?;
    let region = &project.tracks[t].regions[r];
    let (start, end) = (region.start, region.end());
    let mut at: Vec<Ticks> = positions.iter().copied().filter(|p| *p > start && *p < end).collect();
    at.sort_unstable();
    at.dedup();
    if at.is_empty() {
        return Err(invalid("no slice position lies inside the region"));
    }
    if at.len() > MAX_SLICES {
        return Err(EditError::Limit(format!("at most {MAX_SLICES} slices")));
    }
    if project.tracks[t].regions.len() + at.len() > limits::MAX_REGIONS_PER_TRACK {
        return Err(EditError::Limit("too many regions on this track".into()));
    }
    let mut current = id;
    for p in at {
        let mut o = EditOutcome::default();
        crate::timeline_edit::split_regions(project, &[current], p, &mut o)?;
        current = o.created_regions[0];
        outcome.created_regions.push(current);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Bounce and media

/// Adds a rendered file and places it on a new audio track below
/// `source_track`; optionally mutes the regions it was rendered from.
pub(crate) fn bounce_in_place(
    project: &mut Project,
    asset: &MediaAsset,
    start: Ticks,
    source_track: TrackId,
    regions: &[RegionId],
    mute_sources: bool,
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    let index = project
        .track_index(source_track)
        .ok_or(EditError::UnknownTrack(source_track))?;
    let info = asset
        .info
        .clone()
        .ok_or_else(|| invalid("a bounced file must be measured"))?;
    if project.tracks.len() >= limits::MAX_TRACKS {
        return Err(EditError::Limit(format!("at most {} tracks", limits::MAX_TRACKS)));
    }
    if project.media_asset(asset.id).is_some() {
        return Err(invalid("media id already in the pool"));
    }
    if project.media.len() >= limits::MAX_MEDIA {
        return Err(EditError::Limit("too many audio files".into()));
    }
    for id in regions {
        locate(project, *id)?;
    }
    let map = tempo_map(project)?;
    let end = crate::timeline_edit::tick_after_frames(&map, start, info.frames.max(1), info.sample_rate);
    let source = &project.tracks[index];
    let name: String = format!("{} Bounce", source.name)
        .chars()
        .take(limits::MAX_NAME_CHARS)
        .collect();
    let mut track = Track::new(TrackKind::Audio, name, source.color);
    track.volume_db = 0.0;
    let region = Region {
        capture: None,
        midi_events: Vec::new(),
        id: RegionId::new(),
        name: asset.name.clone(),
        start,
        length: end - start,
        muted: false,
        content: RegionContent::audio(Some(asset.id)),
        automation: Vec::new(),
    };
    outcome.created_regions.push(region.id);
    outcome.created_tracks.push(track.id);
    track.regions.push(region);
    project.media.push(asset.clone());
    project.tracks.insert(index + 1, track);
    if mute_sources {
        for t in &mut project.tracks {
            for r in &mut t.regions {
                if regions.contains(&r.id) {
                    r.muted = true;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn relink_all(project: &mut Project, items: &[MediaRelink]) -> Result<(), EditError> {
    if items.is_empty() {
        return Err(invalid("nothing to relink"));
    }
    for item in items {
        if item.path.is_empty() || item.path.chars().count() > limits::MAX_PATH_CHARS {
            return Err(invalid("invalid file path"));
        }
        let asset = project
            .media
            .iter_mut()
            .find(|m| m.id == item.id)
            .ok_or_else(|| invalid("unknown media file"))?;
        asset.path = item.path.clone();
        if item.info.is_some() {
            asset.info = item.info.clone();
        }
    }
    Ok(())
}

/// Media ids referenced by regions or freezes.
pub fn used_media(project: &Project) -> HashSet<MediaId> {
    let mut used = HashSet::new();
    for t in &project.tracks {
        if let Some(f) = &t.freeze {
            used.insert(f.media);
        }
        for r in &t.regions {
            if let RegionContent::Audio { media: Some(id), .. } = r.content {
                used.insert(id);
            }
        }
    }
    used
}

/// Drops pool entries no region or freeze uses (files stay on disk).
pub(crate) fn remove_unused_media(project: &mut Project) -> Result<(), EditError> {
    let used = used_media(project);
    let before = project.media.len();
    project.media.retain(|m| used.contains(&m.id));
    if project.media.len() == before {
        return Err(invalid("every audio file in the pool is in use"));
    }
    Ok(())
}
