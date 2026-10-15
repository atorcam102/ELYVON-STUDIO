//! Integration tests for the on-disk project format and editing session.

use std::path::{Path, PathBuf};

use elyvon_core::{KeyMode, PitchClass, TimeSignature};
use elyvon_project::format::{self, CURRENT_SCHEMA_VERSION};
use elyvon_project::{
    EditCommand, Project, ProjectError, ProjectOrigin, ProjectSession, RegionContent, TrackKind,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Files written by v0.1.0 (schema 1) must keep loading in every later version.
#[test]
fn schema_v1_fixture_loads_with_all_fields() {
    let p = format::load(&fixture("v1-basic.elyvon")).unwrap();
    assert_eq!(p.name, "Fixture Song");
    assert_eq!(p.transport.tempo.bpm(), 98.5);
    assert_eq!(
        p.transport.time_signature,
        TimeSignature::new(6, 8).unwrap()
    );
    assert_eq!(p.transport.key.tonic, PitchClass::FSharp);
    assert_eq!(p.transport.key.mode, KeyMode::Minor);
    assert!(p.transport.loop_range.enabled);
    assert_eq!(p.length_bars, 48);
    assert_eq!(p.tracks.len(), 3);
    let kinds: Vec<_> = p.tracks.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        [TrackKind::Instrument, TrackKind::Audio, TrackKind::Midi]
    );
    assert!(p.tracks[0].soloed && p.tracks[1].muted);
    assert_eq!(p.tracks[0].color.to_hex(), "#5B8DEF");
    match &p.tracks[0].regions[0].content {
        RegionContent::Midi { notes } => assert_eq!(notes.len(), 2),
        other => panic!("unexpected content {other:?}"),
    }
    match &p.tracks[1].regions[0].content {
        RegionContent::Audio { source: Some(s) } => assert_eq!(s.path, "Audio/take1.wav"),
        other => panic!("unexpected content {other:?}"),
    }
}

#[test]
fn encode_decode_round_trip_is_lossless() {
    let original = format::load(&fixture("v1-basic.elyvon")).unwrap();
    let bytes = format::encode(&original, "test");
    assert_eq!(format::decode(&bytes).unwrap(), original);
    let demo = elyvon_project::demo::demo_project();
    assert_eq!(
        format::decode(&format::encode(&demo, "test")).unwrap(),
        demo
    );
}

#[test]
fn encoded_files_carry_the_current_envelope() {
    let bytes = format::encode(&Project::new("X"), "9.9.9");
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["format"], "elyvon-studio-project");
    assert_eq!(v["schemaVersion"], CURRENT_SCHEMA_VERSION);
    assert_eq!(v["savedWith"], "9.9.9");
}

#[test]
fn newer_schema_is_refused_with_clear_error() {
    let text = std::fs::read_to_string(fixture("v1-basic.elyvon")).unwrap();
    let newer = text.replace("\"schemaVersion\": 1", "\"schemaVersion\": 2");
    let err = format::decode(newer.as_bytes()).unwrap_err();
    assert!(matches!(
        err,
        ProjectError::NewerSchema {
            found: 2,
            supported: 1
        }
    ));
    assert!(err.to_string().contains("newer version"));
}

#[test]
fn semantically_invalid_files_are_refused() {
    let text = std::fs::read_to_string(fixture("v1-basic.elyvon")).unwrap();
    for (from, to) in [
        ("\"tempo\": 98.5", "\"tempo\": 5000"),
        ("\"color\": \"#5B8DEF\"", "\"color\": \"blue\""),
        ("\"pitch\": 66", "\"pitch\": 300"),
        ("\"kind\": \"midi\"", "\"kind\": \"video\""),
        ("\"length\": 2880,", "\"length\": 0,"),
        ("\"name\": \"Piano\"", "\"name\": \"\""),
    ] {
        assert!(text.contains(from), "fixture changed: {from}");
        let broken = text.replacen(from, to, 1);
        assert!(
            format::decode(broken.as_bytes()).is_err(),
            "{to} should be rejected"
        );
    }
}

#[test]
fn missing_and_unreadable_files_report_read_errors() {
    let dir = tempfile::tempdir().unwrap();
    let err = format::load(&dir.path().join("nope.elyvon")).unwrap_err();
    assert!(matches!(err, ProjectError::Read { .. }));
    // A directory is not a project file.
    assert!(format::load(dir.path()).is_err());
}

#[test]
fn saving_into_a_missing_directory_fails_without_marking_clean() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectSession::new_empty();
    s.apply(&EditCommand::AddTrack {
        kind: TrackKind::Audio,
        name: None,
        index: None,
    })
    .unwrap();
    let err = s
        .save(Some(&dir.path().join("missing/song.elyvon")), "test")
        .unwrap_err();
    assert!(matches!(err, ProjectError::Write { .. }));
    assert!(s.is_dirty());
}

/// Create → edit → save → reopen keeps every modification.
#[test]
fn modifications_persist_across_save_and_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectSession::new_empty();
    let keys = s
        .apply(&EditCommand::AddTrack {
            kind: TrackKind::Instrument,
            name: Some("Keys".into()),
            index: None,
        })
        .unwrap()
        .created_tracks[0];
    let gtr = s
        .apply(&EditCommand::AddTrack {
            kind: TrackKind::Audio,
            name: None,
            index: None,
        })
        .unwrap()
        .created_tracks[0];
    let region = s
        .apply(&EditCommand::CreateMidiRegion {
            track_id: keys,
            start: 3840,
            length: 7680,
        })
        .unwrap()
        .created_regions[0];
    for cmd in [
        EditCommand::RenameProject {
            name: "My Song".into(),
        },
        EditCommand::SetTempo { bpm: 87.0 },
        EditCommand::SetTimeSignature {
            numerator: 3,
            denominator: 4,
        },
        EditCommand::SetTrackColor {
            id: gtr,
            color: "#112233".parse().unwrap(),
        },
        EditCommand::RenameTrack {
            id: gtr,
            name: "Guitar".into(),
        },
        EditCommand::SetTracksMuted {
            ids: vec![gtr],
            muted: true,
        },
        EditCommand::SetTrackVolume {
            id: keys,
            volume_db: -6.0,
        },
        EditCommand::SetTrackPan { id: keys, pan: 0.5 },
        EditCommand::MoveRegions {
            ids: vec![region],
            delta_ticks: 960,
            track_offset: 0,
        },
        EditCommand::RenameRegion {
            id: region,
            name: "Verse".into(),
        },
    ] {
        s.apply(&cmd).unwrap();
    }
    let path = s.save(Some(&dir.path().join("my-song")), "test").unwrap();
    assert!(!s.is_dirty());

    let reopened = ProjectSession::open(&path).unwrap();
    assert_eq!(reopened.origin(), ProjectOrigin::File);
    assert!(!reopened.is_dirty());
    assert_eq!(reopened.project(), s.project());
    let p = reopened.project();
    assert_eq!(p.name, "My Song");
    assert_eq!(p.tracks[1].name, "Guitar");
    assert!(p.tracks[1].muted);
    assert_eq!(p.tracks[0].regions[0].start, 4800);
    assert_eq!(p.tracks[0].regions[0].name, "Verse");

    // Saving again over the same file replaces it atomically.
    let mut reopened = reopened;
    reopened
        .apply(&EditCommand::RemoveTracks { ids: vec![gtr] })
        .unwrap();
    reopened.save(None, "test").unwrap();
    assert_eq!(
        ProjectSession::open(&path).unwrap().project().tracks.len(),
        1
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temporary files left behind");
}
