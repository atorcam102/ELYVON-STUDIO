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
    // Schema 1 → 2: the audio path moves into the media pool, nothing is lost.
    assert_eq!(p.media.len(), 1);
    assert_eq!(p.media[0].path, "Audio/take1.wav");
    assert_eq!(p.media[0].name, "take1.wav");
    assert!(p.media[0].info.is_none());
    match &p.tracks[1].regions[0].content {
        RegionContent::Audio {
            media: Some(id),
            offset: 0,
            gain_db,
            fade_in: 0,
            fade_out: 0,
            looped: false,
        } => {
            assert_eq!(*id, p.media[0].id);
            assert_eq!(*gain_db, 0.0);
        }
        other => panic!("unexpected content {other:?}"),
    }
    assert_eq!(p.sample_rate, 48_000);
    assert!(p.transport.tempo_changes.is_empty() && p.markers.is_empty());
    assert!(p.tracks.iter().flat_map(|t| &t.regions).all(|r| !r.muted));
}

/// Files written by schema 2 (this phase) keep every new field.
#[test]
fn schema_v2_fixture_loads_tempo_map_markers_and_media() {
    let p = format::load(&fixture("v2-timeline.elyvon")).unwrap();
    assert_eq!(p.sample_rate, 44_100);
    assert_eq!(p.transport.tempo_changes.len(), 2);
    assert_eq!(p.transport.tempo_changes[1].bpm.bpm(), 140.0);
    assert_eq!(p.transport.meter_changes[0].bar, 4);
    assert_eq!(
        p.transport.meter_changes[0].time_signature,
        TimeSignature::new(7, 8).unwrap()
    );
    assert!(p.transport.metronome.enabled);
    assert_eq!(p.transport.count_in_bars, 1);
    assert_eq!(p.transport.pre_roll_bars, 2);
    assert!(p.transport.punch.enabled);
    assert_eq!(p.markers.len(), 2);
    assert_eq!(p.media[0].info.as_ref().unwrap().frames, 88_200);
    let region = &p.tracks[0].regions[0];
    assert!(region.muted);
    match &region.content {
        RegionContent::Audio {
            offset,
            gain_db,
            fade_in,
            fade_out,
            looped,
            ..
        } => {
            assert_eq!(
                (*offset, *fade_in, *fade_out, *looped),
                (441, 100, 2205, true)
            );
            assert_eq!(*gain_db, -4.5);
        }
        other => panic!("unexpected content {other:?}"),
    }
    let bytes = format::encode(&p, "test");
    assert_eq!(format::decode(&bytes).unwrap(), p);
    let map = p.transport.tempo_map().unwrap();
    assert_eq!(map.position(map.bar_start(4)).to_string(), "5.1.000");
}

#[test]
fn v2_semantic_checks_reject_broken_references() {
    let text = std::fs::read_to_string(fixture("v2-timeline.elyvon")).unwrap();
    for (from, to) in [
        ("\"bpm\": 140.0", "\"bpm\": 2000.0"),
        ("\"bar\": 4", "\"bar\": 0"),
        ("\"sampleRate\": 44100", "\"sampleRate\": 12345"),
        ("\"countInBars\": 1", "\"countInBars\": 9"),
        ("\"gainDb\": -4.5", "\"gainDb\": 60.0"),
        (
            "\"media\": \"0b5b8a7e-6a4c-4b8a-9a55-000000000001\"",
            "\"media\": \"0b5b8a7e-6a4c-4b8a-9a55-0000000000ff\"",
        ),
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
    let newer = text.replace("\"schemaVersion\": 1", "\"schemaVersion\": 4");
    let err = format::decode(newer.as_bytes()).unwrap_err();
    assert!(matches!(
        err,
        ProjectError::NewerSchema {
            found: 4,
            supported: CURRENT_SCHEMA_VERSION
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

/// Restart-equivalent: drop every session, reload schema-1, preserve all fields
/// and media references, then recover an unsaved edit without touching the save.
#[test]
fn phase1_file_survives_save_restart_recovery_and_failed_open() {
    let dir = tempfile::tempdir().unwrap();
    let original = format::load(&fixture("v1-basic.elyvon")).unwrap();
    let path = dir.path().join("legacy.elyvon");
    format::save(&path, &original, "0.1.0").unwrap();
    let mut session = ProjectSession::open(&path).unwrap();
    session
        .apply(&EditCommand::RenameProject {
            name: "Unsaved recovery".into(),
        })
        .unwrap();
    elyvon_project::recovery::write(dir.path(), session.project(), Some(&path), "0.1.0").unwrap();
    let unsaved = session.project().clone();
    drop(session);
    assert_eq!(ProjectSession::open(&path).unwrap().project(), &original);
    let (info, recovered) = elyvon_project::recovery::read(dir.path()).unwrap().unwrap();
    assert_eq!(info.original_path, Some(path.display().to_string()));
    assert_eq!(recovered, unsaved);
    let mut restored = ProjectSession::recovered(recovered, Some(path.clone()));
    assert!(restored.is_dirty());
    let bytes = std::fs::read(&path).unwrap();
    assert!(ProjectSession::open(&dir.path().join("missing.elyvon")).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(restored.project(), &unsaved);
    restored.save(None, "0.1.0").unwrap();
    drop(restored);
    assert_eq!(ProjectSession::open(&path).unwrap().project(), &unsaved);
}
