//! Semantic validation of a [`Project`].
//!
//! Everything read from disk passes through [`validate`] before it reaches the
//! session, and every edit is re-validated, so the rest of the application can
//! rely on these invariants.

use std::collections::HashSet;

use elyvon_core::Ticks;

use crate::error::ValidationError;
use crate::model::{MediaId, Project, RegionContent, Track, Transport};

/// Hard limits that keep memory use and file sizes bounded.
pub mod limits {
    pub const MAX_NAME_CHARS: usize = 128;
    pub const MAX_TRACKS: usize = 512;
    pub const MAX_REGIONS_PER_TRACK: usize = 4096;
    pub const MAX_NOTES_PER_REGION: usize = 100_000;
    pub const MAX_LENGTH_BARS: u32 = 9999;
    pub const MAX_PATH_CHARS: usize = 4096;
    /// ~2^40 ticks: hours of music at any tempo, far from overflow.
    pub const MAX_TICKS: u64 = 1 << 40;
    pub const MAX_MARKERS: usize = 1000;
    pub const MAX_MEDIA: usize = 4096;
    pub const MAX_MEDIA_CHANNELS: u16 = 32;
    pub const MIN_MEDIA_RATE: u32 = 1000;
    pub const MAX_MEDIA_RATE: u32 = 768_000;
}

use limits::*;

pub fn validate_name(path: &str, name: &str) -> Result<(), ValidationError> {
    if name.trim().is_empty() {
        return Err(ValidationError::new(path, "must not be empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ValidationError::new(
            path,
            format!("longer than {MAX_NAME_CHARS} characters"),
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(ValidationError::new(path, "contains control characters"));
    }
    Ok(())
}

fn check_ticks(path: &str, value: Ticks) -> Result<(), ValidationError> {
    if value > MAX_TICKS {
        return Err(ValidationError::new(path, "is out of range"));
    }
    Ok(())
}

pub fn validate(project: &Project) -> Result<(), ValidationError> {
    validate_name("name", &project.name)?;

    let t = &project.transport;
    if !t.tempo.is_valid() {
        return Err(ValidationError::new(
            "transport.tempo",
            "must be between 20 and 999 BPM",
        ));
    }
    if !t.time_signature.is_valid() {
        return Err(ValidationError::new(
            "transport.timeSignature",
            "numerator must be 1–32 and denominator 1, 2, 4, 8, 16 or 32",
        ));
    }
    let lr = &t.loop_range;
    check_ticks("transport.loopRange.end", lr.end)?;
    if lr.start >= lr.end {
        return Err(ValidationError::new(
            "transport.loopRange",
            "start must be before end",
        ));
    }
    let punch = &t.punch;
    check_ticks("transport.punch.end", punch.end)?;
    if punch.start >= punch.end {
        return Err(ValidationError::new(
            "transport.punch",
            "start must be before end",
        ));
    }
    for (i, c) in t.tempo_changes.iter().enumerate() {
        check_ticks(&format!("transport.tempoChanges[{i}].tick"), c.tick)?;
    }
    t.tempo_map()
        .map_err(|e| ValidationError::new("transport.tempoChanges", e.to_string()))?;
    let m = &t.metronome;
    if !(Track::MIN_VOLUME_DB..=Track::MAX_VOLUME_DB).contains(&m.volume_db) {
        return Err(ValidationError::new(
            "transport.metronome.volumeDb",
            "must be between -96 and 6 dB",
        ));
    }
    if t.count_in_bars > Transport::MAX_COUNT_IN_BARS {
        return Err(ValidationError::new("transport.countInBars", "must be 0–4"));
    }
    if t.pre_roll_bars > Transport::MAX_PRE_ROLL_BARS {
        return Err(ValidationError::new("transport.preRollBars", "must be 0–4"));
    }
    if !Project::SAMPLE_RATES.contains(&project.sample_rate) {
        return Err(ValidationError::new(
            "sampleRate",
            "must be 44100, 48000, 88200, 96000, 176400 or 192000",
        ));
    }
    if project.markers.len() > MAX_MARKERS {
        return Err(ValidationError::new(
            "markers",
            format!("more than {MAX_MARKERS} markers"),
        ));
    }
    let mut marker_ids = HashSet::with_capacity(project.markers.len());
    for (i, marker) in project.markers.iter().enumerate() {
        validate_name(&format!("markers[{i}].name"), &marker.name)?;
        check_ticks(&format!("markers[{i}].tick"), marker.tick)?;
        if !marker_ids.insert(marker.id) {
            return Err(ValidationError::new(
                format!("markers[{i}].id"),
                "duplicate marker id",
            ));
        }
    }
    let media = validate_media(project)?;
    if project.length_bars == 0 || project.length_bars > MAX_LENGTH_BARS {
        return Err(ValidationError::new(
            "lengthBars",
            format!("must be 1–{MAX_LENGTH_BARS}"),
        ));
    }
    if project.tracks.len() > MAX_TRACKS {
        return Err(ValidationError::new(
            "tracks",
            format!("more than {MAX_TRACKS} tracks"),
        ));
    }

    let mut track_ids = HashSet::with_capacity(project.tracks.len());
    let mut region_ids = HashSet::new();
    for (ti, track) in project.tracks.iter().enumerate() {
        let tp = format!("tracks[{ti}]");
        if !track_ids.insert(track.id) {
            return Err(ValidationError::new(
                format!("{tp}.id"),
                "duplicate track id",
            ));
        }
        validate_track(&tp, track, &media)?;
        crate::comp::validate_track(&tp, track)?;
        crate::comp::validate_freeze(&tp, track, &media)?;
        crate::mixer::validate_strip(&format!("{tp}.strip"), &track.strip)?;
        for (ri, region) in track.regions.iter().enumerate() {
            if !region_ids.insert(region.id) {
                return Err(ValidationError::new(
                    format!("{tp}.regions[{ri}].id"),
                    "duplicate region id",
                ));
            }
        }
    }
    crate::mixer::validate_mixer(project)?;
    crate::automation::validate_project(project)
}

/// Validates the media pool and returns its ids.
fn validate_media(project: &Project) -> Result<HashSet<MediaId>, ValidationError> {
    if project.media.len() > MAX_MEDIA {
        return Err(ValidationError::new(
            "media",
            format!("more than {MAX_MEDIA} files"),
        ));
    }
    let mut ids = HashSet::with_capacity(project.media.len());
    for (i, asset) in project.media.iter().enumerate() {
        let mp = format!("media[{i}]");
        if !ids.insert(asset.id) {
            return Err(ValidationError::new(
                format!("{mp}.id"),
                "duplicate media id",
            ));
        }
        validate_name(&format!("{mp}.name"), &asset.name)?;
        if asset.path.is_empty() || asset.path.chars().count() > MAX_PATH_CHARS {
            return Err(ValidationError::new(
                format!("{mp}.path"),
                "is empty or too long",
            ));
        }
        if let Some(info) = &asset.info {
            if !(MIN_MEDIA_RATE..=MAX_MEDIA_RATE).contains(&info.sample_rate) {
                return Err(ValidationError::new(
                    format!("{mp}.info.sampleRate"),
                    "is out of range",
                ));
            }
            if info.channels == 0 || info.channels > MAX_MEDIA_CHANNELS {
                return Err(ValidationError::new(
                    format!("{mp}.info.channels"),
                    "must be 1–32",
                ));
            }
            if info.frames > MAX_TICKS || info.format.len() > 16 || info.fingerprint.len() > 32 {
                return Err(ValidationError::new(
                    format!("{mp}.info"),
                    "is out of range",
                ));
            }
        }
    }
    Ok(ids)
}

fn validate_track(
    tp: &str,
    track: &Track,
    media: &HashSet<MediaId>,
) -> Result<(), ValidationError> {
    track
        .io
        .validate()
        .map_err(|e| ValidationError::new(format!("{tp}.io"), e))?;
    validate_name(&format!("{tp}.name"), &track.name)?;
    if !(Track::MIN_VOLUME_DB..=Track::MAX_VOLUME_DB).contains(&track.volume_db) {
        return Err(ValidationError::new(
            format!("{tp}.volumeDb"),
            format!(
                "must be between {} and {} dB",
                Track::MIN_VOLUME_DB,
                Track::MAX_VOLUME_DB
            ),
        ));
    }
    if !(-1.0..=1.0).contains(&track.pan) {
        return Err(ValidationError::new(
            format!("{tp}.pan"),
            "must be between -1 and 1",
        ));
    }
    if track.regions.len() > MAX_REGIONS_PER_TRACK {
        return Err(ValidationError::new(
            format!("{tp}.regions"),
            format!("more than {MAX_REGIONS_PER_TRACK} regions"),
        ));
    }
    for (ri, region) in track.regions.iter().enumerate() {
        let rp = format!("{tp}.regions[{ri}]");
        if region.midi_events.len() > MAX_NOTES_PER_REGION
            || region.midi_events.iter().any(|e| {
                e.tick > region.length
                    || !crate::io::validate_midi_bytes(&e.bytes)
                    || crate::midi_edit::is_note_message(&e.bytes)
            })
            || region.midi_events.windows(2).any(|w| w[0].tick > w[1].tick)
        {
            return Err(ValidationError::new(
                format!("{rp}.midiEvents"),
                "invalid, unordered or excessive MIDI events",
            ));
        }
        if !region.midi_events.is_empty() && !matches!(region.content, RegionContent::Midi { .. }) {
            return Err(ValidationError::new(
                format!("{rp}.midiEvents"),
                "MIDI events require a MIDI region",
            ));
        }
        if region.capture.as_ref().is_some_and(|c| c.take == 0) {
            return Err(ValidationError::new(
                format!("{rp}.capture"),
                "take numbers start at one",
            ));
        }

        validate_name(&format!("{rp}.name"), &region.name)?;
        check_ticks(&format!("{rp}.start"), region.start)?;
        check_ticks(&format!("{rp}.length"), region.length)?;
        if region.length == 0 {
            return Err(ValidationError::new(
                format!("{rp}.length"),
                "must be greater than zero",
            ));
        }
        if !track.kind.accepts(&region.content) {
            return Err(ValidationError::new(
                format!("{rp}.content"),
                format!("is not allowed on a {:?} track", track.kind),
            ));
        }
        match &region.content {
            RegionContent::Audio {
                media: id,
                offset,
                gain_db,
                fade_in,
                fade_out,
                looped,
                reversed,
                ..
            } => {
                if *looped && *reversed {
                    return Err(ValidationError::new(
                        format!("{rp}.content.reversed"),
                        "a region cannot be both looped and reversed",
                    ));
                }
                if let Some(id) = id
                    && !media.contains(id)
                {
                    return Err(ValidationError::new(
                        format!("{rp}.content.media"),
                        "refers to a file that is not in the media pool",
                    ));
                }
                if !(RegionContent::MIN_GAIN_DB..=RegionContent::MAX_GAIN_DB).contains(gain_db) {
                    return Err(ValidationError::new(
                        format!("{rp}.content.gainDb"),
                        "must be between -96 and 24 dB",
                    ));
                }
                for (name, v) in [
                    ("offset", offset),
                    ("fadeIn", fade_in),
                    ("fadeOut", fade_out),
                ] {
                    check_ticks(&format!("{rp}.content.{name}"), *v)?;
                }
            }
            RegionContent::Midi { notes } => {
                if notes.len() > MAX_NOTES_PER_REGION {
                    return Err(ValidationError::new(
                        format!("{rp}.content.notes"),
                        format!("more than {MAX_NOTES_PER_REGION} notes"),
                    ));
                }
                let mut ids = HashSet::with_capacity(notes.len());
                for (ni, n) in notes.iter().enumerate() {
                    let np = format!("{rp}.content.notes[{ni}]");
                    if n.id == 0 || !ids.insert(n.id) {
                        return Err(ValidationError::new(
                            format!("{np}.id"),
                            "note ids must be unique and non-zero",
                        ));
                    }
                    if n.channel > 15 {
                        return Err(ValidationError::new(
                            format!("{np}.channel"),
                            "must be 0–15",
                        ));
                    }
                    if i64::from(n.quantize_offset) > n.start as i64 {
                        return Err(ValidationError::new(
                            format!("{np}.quantizeOffset"),
                            "the unquantised position would be before the region",
                        ));
                    }
                    if n.pitch > 127 {
                        return Err(ValidationError::new(format!("{np}.pitch"), "must be 0–127"));
                    }
                    if n.velocity == 0 || n.velocity > 127 {
                        return Err(ValidationError::new(
                            format!("{np}.velocity"),
                            "must be 1–127",
                        ));
                    }
                    check_ticks(&format!("{np}.start"), n.start)?;
                    check_ticks(&format!("{np}.length"), n.length)?;
                    if n.length == 0 {
                        return Err(ValidationError::new(
                            format!("{np}.length"),
                            "must be greater than zero",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn project_with_track() -> Project {
        let mut p = Project::new("Song");
        p.tracks
            .push(Track::new(TrackKind::Instrument, "Keys", TRACK_PALETTE[0]));
        p
    }

    #[test]
    fn default_project_is_valid() {
        validate(&Project::new("Song")).unwrap();
    }

    #[test]
    fn rejects_bad_names() {
        let mut p = Project::new("  ");
        assert_eq!(validate(&p).unwrap_err().path, "name");
        p.name = "a".repeat(limits::MAX_NAME_CHARS + 1);
        assert!(validate(&p).is_err());
        p.name = "bad\u{0007}".into();
        assert!(validate(&p).is_err());
    }

    #[test]
    fn rejects_out_of_range_mix_values() {
        let mut p = project_with_track();
        p.tracks[0].volume_db = 12.0;
        assert_eq!(validate(&p).unwrap_err().path, "tracks[0].volumeDb");
        p.tracks[0].volume_db = f64::NAN;
        assert!(validate(&p).is_err());
        p.tracks[0].volume_db = 0.0;
        p.tracks[0].pan = -1.5;
        assert_eq!(validate(&p).unwrap_err().path, "tracks[0].pan");
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut p = project_with_track();
        let dup = p.tracks[0].clone();
        p.tracks.push(dup);
        assert_eq!(validate(&p).unwrap_err().message, "duplicate track id");
    }

    #[test]
    fn rejects_region_on_incompatible_track() {
        let mut p = project_with_track();
        p.tracks[0].regions.push(Region {
            capture: None,
            midi_events: Vec::new(),
            id: RegionId::new(),
            name: "Clip".into(),
            start: 0,
            length: 960,
            muted: false,
            content: RegionContent::audio(None),
            automation: Vec::new(),
        });
        assert!(validate(&p).unwrap_err().message.contains("not allowed"));
    }

    #[test]
    fn rejects_invalid_notes() {
        let mut p = project_with_track();
        p.tracks[0].regions.push(Region {
            capture: None,
            midi_events: Vec::new(),
            id: RegionId::new(),
            name: "Notes".into(),
            start: 0,
            length: 960,
            muted: false,
            content: RegionContent::Midi {
                notes: vec![MidiNote {
                    id: 1,
                    ..MidiNote::new(200, 100, 0, 10)
                }],
            },
            automation: Vec::new(),
        });
        assert!(validate(&p).unwrap_err().path.ends_with("pitch"));
    }

    #[test]
    fn rejects_empty_loop() {
        let mut p = Project::new("Song");
        p.transport.loop_range.end = p.transport.loop_range.start;
        assert!(validate(&p).is_err());
    }
}
