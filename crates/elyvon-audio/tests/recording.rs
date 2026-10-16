use elyvon_audio::recording::*;
use elyvon_audio::transport::PlayState;
use elyvon_project::TrackId;
use std::sync::atomic::Ordering;
fn config() -> CaptureConfig {
    CaptureConfig {
        rate: 48000,
        input_channels: 2,
        start_frame: 0,
        punch: None,
        cycle: None,
        compensation_frames: 0,
        monitor_only: false,
        max_bytes: None,
    }
}
fn route(ch: u16) -> CaptureRoute {
    CaptureRoute {
        track: TrackId::new(),
        input: ch,
        channels: 1,
        armed: true,
        monitor: false,
        gain: 1.0,
    }
}
#[test]
fn multitrack_record_stop_decode_restart_preserves_samples_and_bwf() {
    let dir = tempfile::tempdir().unwrap();
    let routes = vec![route(0), route(1)];
    let (mut tap, writer) = RecordingWriter::start(dir.path(), config(), routes, None).unwrap();
    let input: Vec<f32> = (0..4800)
        .flat_map(|i| [i as f32 / 10000.0, -i as f32 / 12000.0])
        .collect();
    for (i, chunk) in input.chunks(512).enumerate() {
        tap.process(chunk, (i * 256) as u64, PlayState::Playing, 0);
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    drop(tap);
    let result = writer.finish(false);
    assert_eq!(
        result.state,
        RecordingState::Finalized,
        "{:?}",
        result.error
    );
    assert_eq!(result.takes.len(), 2);
    for (ch, take) in result.takes.iter().enumerate() {
        let decoded =
            elyvon_audio::media::decode_file(std::path::Path::new(&take.path), 10000).unwrap();
        assert_eq!(
            decoded.planes[0],
            input
                .iter()
                .skip(ch)
                .step_by(2)
                .copied()
                .collect::<Vec<_>>()
        );
        assert_eq!(take.frames, 4800);
        assert!(!take.recoverable);
        let data = std::fs::read(&take.path).unwrap();
        assert_eq!(&data[12..16], b"bext");
    }
    let mut session = elyvon_project::ProjectSession::new_empty();
    let mut assets = Vec::new();
    let mut placements = Vec::new();
    let mut buffers = std::collections::HashMap::new();
    for take in &result.takes {
        let track = session
            .apply(&elyvon_project::EditCommand::AddTrack {
                kind: elyvon_project::TrackKind::Audio,
                name: None,
                index: None,
            })
            .unwrap()
            .created_tracks[0];
        let buffer = elyvon_audio::media::MediaStore::load_file(
            std::path::Path::new(&take.path),
            48000,
            8 << 20,
        )
        .unwrap();
        let id = elyvon_project::MediaId::new();
        assets.push(elyvon_project::MediaAsset {
            id,
            name: "Recorded take".into(),
            path: take.path.clone(),
            info: Some(buffer.info.clone()),
        });
        placements.push(elyvon_project::AudioPlacement {
            media_id: id,
            track: elyvon_project::PlacementTrack::Existing { id: track },
            start: 0,
        });
        buffers.insert(id, buffer);
    }
    session
        .apply(&elyvon_project::EditCommand::ImportTakes {
            assets,
            placements,
            recorded_at: result.takes[0].recorded_at,
        })
        .unwrap();
    let path = dir.path().join("recorded.elyvon");
    session.save(Some(&path), "0.1.0").unwrap();
    let reopened = elyvon_project::ProjectSession::open(&path).unwrap();
    assert_eq!(session.project(), reopened.project());
    let lookup = |id| buffers.get(&id).cloned();
    let render = elyvon_audio::render::render(
        reopened.project(),
        &lookup,
        elyvon_audio::render::RenderRequest::whole_song(48000),
        None,
    );
    let export = dir.path().join("recorded-mix.wav");
    let report = elyvon_audio::render::export_project(
        &export,
        reopened.project(),
        &lookup,
        elyvon_audio::render::RenderRequest::whole_song(48000),
    )
    .unwrap();
    assert_eq!(report.checksum, render.report.checksum);
    let decode = elyvon_audio::media::decode_file(&export, 10000).unwrap();
    assert_eq!(decode.planes[0], render.channel(0));
    // A second capture can open the same directory without overwriting takes.
    let (tap, writer) = RecordingWriter::start(dir.path(), config(), vec![route(0)], None).unwrap();
    drop(tap);
    assert_eq!(writer.finish(true).state, RecordingState::Cancelled);
}
#[test]
fn countin_preroll_punch_cycle_and_compensation_gate_capture() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = CaptureConfig {
        start_frame: 4,
        punch: Some((4, 8)),
        cycle: Some((0, 8)),
        compensation_frames: 2,
        ..config()
    };
    let (mut tap, writer) = RecordingWriter::start(dir.path(), cfg, vec![route(0)], None).unwrap();
    tap.process(&[0.2; 16], 0, PlayState::CountIn, 0);
    tap.process(&[0.3; 16], 2, PlayState::Playing, 0); // capture frames 0..8, only 4..8 recorded
    tap.process(&[0.4; 16], 0, PlayState::Playing, 1); // second loop, same punch window
    drop(tap);
    let r = writer.finish(false);
    assert_eq!(r.state, RecordingState::Finalized);
    assert_eq!(r.takes.len(), 2);
    assert!(r.takes.iter().all(|t| t.start_frame == 4 && t.frames == 4));
    assert_eq!(r.takes[1].cycle, 1);
}
#[test]
fn disk_full_and_disconnect_leave_recoverable_channel_aligned_takes() {
    for fault in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let cfg = CaptureConfig {
            max_bytes: (!fault).then_some(1025),
            ..config()
        };
        let (mut tap, writer) =
            RecordingWriter::start(dir.path(), cfg, vec![route(0)], None).unwrap();
        tap.process(&[1.2; 2048], 0, PlayState::Playing, 0);
        if fault {
            tap.health.disconnected.store(true, Ordering::Release);
        }
        drop(tap);
        let r = writer.finish(false);
        assert!(matches!(
            r.state,
            RecordingState::Failed | RecordingState::Interrupted
        ));
        assert!(r.error.is_some());
        assert!(r.clipped_frames > 0);
        for take in &r.takes {
            assert!(take.recoverable);
            let path = std::path::Path::new(&take.path);
            let recovered = recover_take(&path.with_extension("json")).unwrap();
            let decoded =
                elyvon_audio::media::decode_file(std::path::Path::new(&recovered.path), 10000)
                    .unwrap();
            assert_eq!(decoded.info.frames, recovered.frames);
            assert!(recovered.frames > 0);
        }
    }
}
#[test]
fn invalid_routes_and_unwritable_destination_fail_before_capture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    std::fs::write(&path, b"x").unwrap();
    assert!(RecordingWriter::start(&path, config(), vec![route(0)], None).is_err());
    assert!(RecordingWriter::start(dir.path(), config(), vec![route(31)], None).is_err());
}

#[test]
fn compensation_trims_samples_before_the_project_origin() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = CaptureConfig {
        input_channels: 1,
        compensation_frames: 2,
        ..config()
    };
    let (mut tap, writer) = RecordingWriter::start(dir.path(), cfg, vec![route(0)], None).unwrap();
    tap.process(&[0.1, 0.2, 0.3, 0.4], 0, PlayState::Playing, 0);
    drop(tap);
    let report = writer.finish(false);
    assert_eq!(report.takes[0].start_frame, 0);
    assert_eq!(report.takes[0].frames, 2);
    let decoded =
        elyvon_audio::media::decode_file(std::path::Path::new(&report.takes[0].path), 10).unwrap();
    assert_eq!(decoded.planes[0], [0.3, 0.4]);
}
