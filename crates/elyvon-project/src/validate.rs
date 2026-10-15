//! Semantic validation of a [`Project`].
//!
//! Everything read from disk passes through [`validate`] before it reaches the
//! session, and every edit is re-validated, so the rest of the application can
//! rely on these invariants.

use std::collections::HashSet;

use elyvon_core::Ticks;

use crate::error::ValidationError;
use crate::model::{Project, RegionContent, Track};

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
        validate_track(&tp, track)?;
        for (ri, region) in track.regions.iter().enumerate() {
            if !region_ids.insert(region.id) {
                return Err(ValidationError::new(
                    format!("{tp}.regions[{ri}].id"),
                    "duplicate region id",
                ));
            }
        }
    }
    Ok(())
}

fn validate_track(tp: &str, track: &Track) -> Result<(), ValidationError> {
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
            RegionContent::Audio { source } => {
                if let Some(src) = source
                    && (src.path.is_empty() || src.path.chars().count() > MAX_PATH_CHARS)
                {
                    return Err(ValidationError::new(
                        format!("{rp}.content.source.path"),
                        "is empty or too long",
                    ));
                }
            }
            RegionContent::Midi { notes } => {
                if notes.len() > MAX_NOTES_PER_REGION {
                    return Err(ValidationError::new(
                        format!("{rp}.content.notes"),
                        format!("more than {MAX_NOTES_PER_REGION} notes"),
                    ));
                }
                for (ni, n) in notes.iter().enumerate() {
                    let np = format!("{rp}.content.notes[{ni}]");
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
            id: RegionId::new(),
            name: "Clip".into(),
            start: 0,
            length: 960,
            content: RegionContent::Audio { source: None },
        });
        assert!(validate(&p).unwrap_err().message.contains("not allowed"));
    }

    #[test]
    fn rejects_invalid_notes() {
        let mut p = project_with_track();
        p.tracks[0].regions.push(Region {
            id: RegionId::new(),
            name: "Notes".into(),
            start: 0,
            length: 960,
            content: RegionContent::Midi {
                notes: vec![MidiNote {
                    pitch: 200,
                    velocity: 100,
                    start: 0,
                    length: 10,
                }],
            },
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
