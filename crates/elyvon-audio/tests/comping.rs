//! Phase 11 end to end: a cycle recording through the real capture writer,
//! a comp of the three takes, export, save/reopen, undo and flatten — all
//! verified on the rendered samples. The take files are never modified.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use elyvon_audio::media::{MediaBuffer, MediaStore};
use elyvon_audio::recording::{CaptureConfig, CaptureRoute, RecordingState, RecordingWriter};
use elyvon_audio::render::{self, RenderRequest};
use elyvon_audio::transport::PlayState;
use elyvon_project::{
    AudioPlacement, EditCommand, MediaAsset, MediaId, PlacementTrack, Project, ProjectSession,
    RegionContent, TrackKind,
};

const RATE: u32 = 48_000;
/// 1 s cycle at 120 BPM (1920 ticks).
const CYCLE: u64 = 48_000;

fn fnv(path: &Path) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in std::fs::read(path).unwrap() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Distinct material per pass: a sine whose frequency identifies the take.
fn pass_signal(pass: usize, frame: u64) -> f32 {
    let f = [220.0, 330.0, 495.0][pass];
    (0.4 * (2.0 * std::f64::consts::PI * f * frame as f64 / f64::from(RATE)).sin()) as f32
}

fn render_all(project: &Project, media: &HashMap<MediaId, Arc<MediaBuffer>>) -> Vec<f32> {
    let lookup = |id| media.get(&id).cloned();
    render::render(project, &lookup, RenderRequest::whole_song(RATE), None).channel(0)
}

#[test]
fn cycle_takes_comp_export_reopen_undo_and_flatten() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = ProjectSession::new_empty();
    let track = session
        .apply(&EditCommand::AddTrack {
            kind: TrackKind::Audio,
            name: Some("Vocal".into()),
            index: None,
        })
        .unwrap()
        .created_tracks[0];
    // Mono takes at the centre play at unity with the compensated law.
    session
        .apply(&EditCommand::SetPanLaw {
            law: elyvon_project::mixer::PanLaw::ConstantPowerCompensated,
        })
        .unwrap();
    session
        .apply(&EditCommand::SetLoop {
            enabled: true,
            start: 0,
            end: 1920,
        })
        .unwrap();

    // Three passes of the cycle through the capture writer.
    let config = CaptureConfig {
        rate: RATE,
        input_channels: 1,
        start_frame: 0,
        punch: None,
        cycle: Some((0, CYCLE)),
        compensation_frames: 0,
        monitor_only: false,
        max_bytes: None,
    };
    let route = CaptureRoute {
        track,
        input: 0,
        channels: 1,
        armed: true,
        monitor: false,
        gain: 1.0,
    };
    let (mut tap, writer) = RecordingWriter::start(dir.path(), config, vec![route], None).unwrap();
    for pass in 0..3usize {
        let mut pos = 0u64;
        while pos < CYCLE {
            let n = 512.min(CYCLE - pos);
            let block: Vec<f32> = (0..n).map(|i| pass_signal(pass, pos + i)).collect();
            tap.process(&block, pos, PlayState::Playing, pass as u64);
            pos += n;
            // Real-time pacing: the capture queue holds 170 ms.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    drop(tap);
    let report = writer.finish(false);
    assert_eq!(
        report.state,
        RecordingState::Finalized,
        "{:?}",
        report.error
    );
    assert_eq!(report.takes.len(), 3, "one take per cycle pass");

    // Commit the takes exactly as the desktop shell does.
    let mut media: HashMap<MediaId, Arc<MediaBuffer>> = HashMap::new();
    let mut assets = Vec::new();
    let mut placements = Vec::new();
    let mut hashes = Vec::new();
    for (n, take) in report.takes.iter().enumerate() {
        let path = Path::new(&take.path);
        hashes.push((path.to_owned(), fnv(path)));
        let buffer = MediaStore::load_file(path, RATE, 64 << 20).unwrap();
        let id = MediaId::new();
        assets.push(MediaAsset {
            id,
            name: format!("Take {}", n + 1),
            path: take.path.clone(),
            info: Some(buffer.info.clone()),
        });
        placements.push(AudioPlacement {
            media_id: id,
            track: PlacementTrack::Existing { id: track },
            start: 0,
        });
        media.insert(id, buffer);
    }
    session
        .apply(&EditCommand::ImportTakes {
            assets,
            placements,
            recorded_at: report.takes[0].recorded_at,
        })
        .unwrap();
    let regions = session.project().tracks[0].regions.clone();
    assert_eq!(regions.len(), 3);
    let folder = regions[0].capture.as_ref().unwrap().folder;
    assert!(
        regions
            .iter()
            .all(|r| r.capture.as_ref().unwrap().folder == folder)
    );
    let take_of =
        |t: u32| -> Vec<f32> { (0..CYCLE).map(|f| pass_signal(t as usize - 1, f)).collect() };

    // Without a comp the last take plays.
    let last = render_all(session.project(), &media);
    for t in &report.takes {
        eprintln!(
            "take cycle {} start {} frames {}",
            t.cycle, t.start_frame, t.frames
        );
    }
    if let Some(i) = last
        .iter()
        .zip(take_of(3))
        .position(|(a, b)| (a - b).abs() >= 1e-4)
    {
        eprintln!(
            "first mismatch at {i}: {} vs {}; len {}",
            last[i],
            take_of(3)[i],
            last.len()
        );
    }
    assert!(
        last.iter()
            .zip(take_of(3))
            .all(|(a, b)| (a - b).abs() < 1e-4)
    );

    // Comp: take 1 for the first third, take 2 for the second, take 3 after.
    session
        .apply(&EditCommand::CompTake {
            track_id: track,
            folder,
            take: 1,
            start: 0,
            end: 640,
        })
        .unwrap();
    session
        .apply(&EditCommand::CompTake {
            track_id: track,
            folder,
            take: 2,
            start: 640,
            end: 1280,
        })
        .unwrap();
    let comp = session.project().tracks[0].comps[0].clone();
    assert_eq!(
        comp.segments
            .iter()
            .map(|s| (s.start, s.end, s.take))
            .collect::<Vec<_>>(),
        [(0, 640, 1), (640, 1280, 2), (1280, 1920, 3)]
    );
    assert!(
        session.project().tracks[0].regions.iter().all(|r| r.muted),
        "takes kept, muted"
    );
    let comped = render_all(session.project(), &media);
    // Outside the 10 ms crossfades each part is exactly its take (16-bit WAV).
    // 5 ms each side, rounded up to whole ticks (10 ticks = 250 frames).
    let half = 260u64;
    let third = CYCLE / 3;
    for (take, a, b) in [
        (1u32, 0, third - half),
        (2, third + half, 2 * third - half),
        (3, 2 * third + half, CYCLE),
    ] {
        let reference = take_of(take);
        for f in (a..b).step_by(7) {
            let f = f as usize;
            assert!(
                (comped[f] - reference[f]).abs() < 1e-4,
                "take {take} at frame {f}: {} vs {}",
                comped[f],
                reference[f]
            );
        }
    }
    // The transitions are continuous: no jump larger than the material's slope.
    let max_slope = 0.4 * 2.0 * std::f32::consts::PI * 495.0 / RATE as f32;
    let step = comped
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(
        step < max_slope * 1.5,
        "smooth comp transitions: {step} (slope {max_slope})"
    );

    // Export the comp and read it back.
    let wav = dir.path().join("comp.wav");
    let lookup = |id| media.get(&id).cloned();
    render::export_project(
        &wav,
        session.project(),
        &lookup,
        RenderRequest::whole_song(RATE),
    )
    .unwrap();
    let decoded = elyvon_audio::media::decode_file(&wav, 1 << 30).unwrap();
    assert_eq!(decoded.planes[0], comped, "exported comp equals the render");

    // Save, reopen and render again.
    let file = dir.path().join("comp.elyvon");
    session.save(Some(&file), "test").unwrap();
    let reopened = ProjectSession::open(&file).unwrap();
    assert_eq!(reopened.project(), session.project());
    assert_eq!(render_all(reopened.project(), &media), comped);

    // Undo both comp steps: back to the last take; redo restores the comp.
    assert!(session.undo() && session.undo());
    assert!(session.project().tracks[0].comps.is_empty());
    assert_eq!(render_all(session.project(), &media), last);
    assert!(session.redo() && session.redo());
    assert_eq!(render_all(session.project(), &media), comped);

    // Flatten: ordinary regions that sound identical; unflatten reverses it.
    let created = session
        .apply(&EditCommand::FlattenComp {
            track_id: track,
            folder,
        })
        .unwrap()
        .created_regions;
    assert_eq!(created.len(), 3);
    assert_eq!(
        session.project().tracks[0].regions.len(),
        6,
        "the takes are kept"
    );
    assert_eq!(
        render_all(session.project(), &media),
        comped,
        "flattened comp sounds the same"
    );
    // A flattened comp cannot be edited until it is unflattened.
    assert!(
        session
            .apply(&EditCommand::CompTake {
                track_id: track,
                folder,
                take: 3,
                start: 0,
                end: 100
            })
            .is_err()
    );
    session
        .apply(&EditCommand::UnflattenComp {
            track_id: track,
            folder,
        })
        .unwrap();
    assert_eq!(session.project().tracks[0].regions.len(), 3);
    assert_eq!(render_all(session.project(), &media), comped);

    // Choosing one whole take removes the comp; the files never changed.
    session
        .apply(&EditCommand::SelectTake {
            track_id: track,
            folder,
            take: 2,
        })
        .unwrap();
    let two = render_all(session.project(), &media);
    assert!(
        two.iter()
            .zip(take_of(2))
            .all(|(a, b)| (a - b).abs() < 1e-4)
    );
    for (path, hash) in hashes {
        assert_eq!(fnv(&path), hash, "{} unchanged", path.display());
    }
    let r = &session.project().tracks[0].regions[0];
    assert!(matches!(r.content, RegionContent::Audio { .. }));
}
