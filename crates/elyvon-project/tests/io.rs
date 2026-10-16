use elyvon_project::{
    AudioPlacement, EditCommand, MediaAsset, MediaId, MediaInfo, PlacementTrack, ProjectSession,
    RegionContent, TrackIo, TrackKind, format,
    io::{MidiImportRegion, StoredMidiEvent},
};
#[test]
fn recorded_take_folders_and_routing_roundtrip_with_undo_and_safe_migrations() {
    let mut session = ProjectSession::new_empty();
    let track = session
        .apply(&EditCommand::AddTrack {
            kind: TrackKind::Audio,
            name: None,
            index: None,
        })
        .unwrap()
        .created_tracks[0];
    let io = TrackIo {
        armed: true,
        input_channel: 2,
        input_channels: 2,
        monitor: true,
        ..TrackIo::default()
    };
    session
        .apply(&EditCommand::SetTrackIo {
            id: track,
            io: io.clone(),
        })
        .unwrap();
    let mut assets = Vec::new();
    let mut placements = Vec::new();
    for i in 0..2 {
        let id = MediaId::new();
        assets.push(MediaAsset {
            id,
            name: format!("Take {i}"),
            path: format!("/recordings/take-{i}.wav"),
            info: Some(MediaInfo {
                sample_rate: 48000,
                channels: 2,
                frames: 4800,
                format: "wav".into(),
                size_bytes: 38444,
                fingerprint: "original".into(),
            }),
        });
        placements.push(AudioPlacement {
            media_id: id,
            track: PlacementTrack::Existing { id: track },
            start: 0,
        });
    }
    session
        .apply(&EditCommand::ImportTakes {
            assets,
            placements,
            recorded_at: 1792152000,
        })
        .unwrap();
    let regions = &session.project().tracks[0].regions;
    let folder = regions[0].capture.as_ref().unwrap().folder;
    assert!(regions[0].muted);
    assert!(!regions[1].muted);
    assert_eq!(regions[1].capture.as_ref().unwrap().folder, folder);
    session
        .apply(&EditCommand::SelectTake {
            track_id: track,
            folder,
            take: 1,
        })
        .unwrap();
    assert!(!session.project().tracks[0].regions[0].muted);
    session.undo();
    assert!(session.project().tracks[0].regions[0].muted);
    session.redo();
    let bytes = format::encode(session.project(), "0.1.0");
    let reopened = format::decode(&bytes).unwrap();
    assert_eq!(&reopened, session.project());
    assert_eq!(reopened.tracks[0].io, io);
    for name in ["v1-basic.elyvon", "v2-timeline.elyvon"] {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap();
        let p = format::decode(&bytes).unwrap();
        assert!(p.tracks.iter().all(|t| t.io == TrackIo::default()));
        assert_eq!(format::decode(&format::encode(&p, "0.1.0")).unwrap(), p);
    }
}
#[test]
fn midi_notes_and_controllers_survive_split_trim_save_and_routing_fallback() {
    let events = vec![
        StoredMidiEvent {
            tick: 0,
            bytes: vec![0x92, 60, 100],
        },
        StoredMidiEvent {
            tick: 240,
            bytes: vec![0xb2, 64, 127],
        },
        StoredMidiEvent {
            tick: 960,
            bytes: vec![0x82, 60, 0],
        },
        StoredMidiEvent {
            tick: 1200,
            bytes: vec![0xb2, 64, 0],
        },
    ];
    let original = events.clone();
    let mut session = ProjectSession::new_empty();
    let out = session
        .apply(&EditCommand::ImportMidi {
            regions: vec![MidiImportRegion {
                name: "Keys".into(),
                track_id: None,
                start: 0,
                length: 1600,
                notes: vec![],
                events,
            }],
            tempo: None,
            tempo_changes: vec![],
            time_signature: None,
            meter_changes: vec![],
        })
        .unwrap();
    let track = out.created_tracks[0];
    session
        .apply(&EditCommand::SetTrackIo {
            id: track,
            io: TrackIo {
                midi_input: Some("input:absent#0".into()),
                midi_output: Some("output:absent#0".into()),
                midi_channel: Some(2),
                output_channel: None,
                ..TrackIo::default()
            },
        })
        .unwrap();
    let out = session
        .apply(&EditCommand::SplitRegions {
            ids: out.created_regions,
            at: 480,
        })
        .unwrap();
    let right = out.created_regions[0];
    // Schema 4: the note pair became an editable note (channel kept) and
    // the crossing note continues in the right half; controller state is
    // chased to the split point.
    let right_region = &session.project().tracks[0].regions[1];
    let RegionContent::Midi { notes } = &right_region.content else {
        panic!("MIDI region expected");
    };
    assert!(
        notes
            .iter()
            .any(|n| (n.start, n.length, n.pitch, n.velocity, n.channel) == (0, 480, 60, 100, 2))
    );
    assert!(
        right_region
            .midi_events
            .iter()
            .any(|e| e.tick == 0 && e.bytes == [0xb2, 64, 127])
    );
    assert!(
        right_region
            .midi_events
            .iter()
            .all(|e| e.bytes[0] & 0xf0 != 0x90 && e.bytes[0] & 0xf0 != 0x80)
    );
    session
        .apply(&EditCommand::TrimRegion {
            id: right,
            start: 600,
            end: 1200,
        })
        .unwrap();
    let encoded = format::encode(session.project(), "0.1.0");
    assert_eq!(format::decode(&encoded).unwrap(), *session.project());
    assert_eq!(original[0].bytes, [0x92, 60, 100]);
}
