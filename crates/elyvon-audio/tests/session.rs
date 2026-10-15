//! Phase 05 acceptance: a project with more than 12 real audio clips decoded
//! from files plays the right audio, exports a diagnostic render, keeps the
//! source files untouched through edits, and reopens to the same render.
//! Also: waveform columns come from the real data, and large sessions stay
//! fast to plan and to zoom.

// Test helpers outside `#[test]` functions may panic on failure.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use common::*;
use elyvon_audio::media::{MediaBuffer, MediaStore, decode_file, resolve_path};
use elyvon_audio::plan::build_plan;
use elyvon_audio::render::{RenderRequest, Rendered, export, render};
use elyvon_audio::waveform::region_columns;
use elyvon_project::{
    AudioPlacement, EditCommand, MediaAsset, MediaId, PlacementTrack, Project, RegionId, TrackId,
    TrackKind, format,
};

const RATE: u32 = 48_000;
/// At 120 BPM one second is 1920 ticks.
const SEC: u64 = 1920;

struct Files {
    dir: tempfile::TempDir,
    paths: Vec<PathBuf>,
}

fn store_for(project: &Project, base: Option<&Path>) -> HashMap<MediaId, Arc<MediaBuffer>> {
    project
        .media
        .iter()
        .filter_map(|a| {
            let path = resolve_path(&a.path, base);
            MediaStore::load_file(&path, RATE, 1 << 30)
                .ok()
                .map(|b| (a.id, b))
        })
        .collect()
}

fn import(project: &mut Project, path: &Path, track: TrackId, start: u64) -> RegionId {
    let buf = MediaStore::load_file(path, RATE, 1 << 30).unwrap();
    let id = MediaId::new();
    elyvon_project::edit::apply(
        project,
        &EditCommand::ImportAudio {
            assets: vec![MediaAsset {
                id,
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                path: path.display().to_string(),
                info: Some(buf.info.clone()),
            }],
            placements: vec![AudioPlacement {
                media_id: id,
                track: PlacementTrack::Existing { id: track },
                start,
            }],
        },
    )
    .unwrap()
    .created_regions[0]
}

fn add_track(project: &mut Project, pan: f64) -> TrackId {
    let id = elyvon_project::edit::apply(
        project,
        &EditCommand::AddTrack {
            kind: TrackKind::Audio,
            name: None,
            index: None,
        },
    )
    .unwrap()
    .created_tracks[0];
    elyvon_project::edit::apply(project, &EditCommand::SetTrackPan { id, pan }).unwrap();
    id
}

fn do_render(project: &Project, media: &HashMap<MediaId, Arc<MediaBuffer>>) -> Rendered {
    let lookup = |id| media.get(&id).cloned();
    render(project, &lookup, RenderRequest::whole_song(RATE), None)
}

/// Eight exact float-WAV sines for the first 8 s (left channel), then
/// 16-bit WAV, AIFF, FLAC (44.1 kHz, resampled), MP3 and Ogg Vorbis.
fn build() -> (Project, Files, Vec<(u64, f64, f64)>) {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    let mut project = Project::new("Twelve Clips");
    let left_tracks: Vec<TrackId> = (0..4).map(|_| add_track(&mut project, -1.0)).collect();
    let stereo = add_track(&mut project, 0.0);
    let mut exact = Vec::new();
    for k in 0..8u64 {
        let freq = 220.0 + 55.0 * k as f64;
        let amp = 0.05 + 0.01 * k as f64;
        let path = dir.path().join(format!("sine{k}.wav"));
        write_wav(&path, &[sine(freq, amp, RATE, RATE as usize)], RATE, 32);
        let start = k * SEC / 2 + k * 37; // overlapping across tracks, odd offsets
        import(&mut project, &path, left_tracks[(k % 4) as usize], start);
        exact.push((start, freq, amp));
        paths.push(path);
    }
    let w16 = dir.path().join("stereo16.wav");
    write_wav(
        &w16,
        &[
            sine(330.0, 0.2, RATE, RATE as usize),
            sine(495.0, 0.1, RATE, RATE as usize),
        ],
        RATE,
        16,
    );
    import(&mut project, &w16, stereo, 8 * SEC);
    let aiff = dir.path().join("mono.aiff");
    write_aiff(&aiff, &[sine(600.0, 0.3, RATE, RATE as usize)], RATE);
    import(&mut project, &aiff, left_tracks[0], 9 * SEC);
    for (i, name) in ["tone.flac", "tone.mp3", "tone.ogg"]
        .into_iter()
        .enumerate()
    {
        let copy = dir.path().join(name);
        std::fs::copy(fixture(name), &copy).unwrap();
        import(&mut project, &copy, stereo, 10 * SEC + i as u64 * SEC);
        paths.push(copy);
    }
    paths.extend([w16, aiff]);
    // A muted extra clip must not be heard.
    let muted = import(&mut project, &paths[0], left_tracks[1], 13 * SEC);
    elyvon_project::edit::apply(
        &mut project,
        &EditCommand::SetRegionsMuted {
            ids: vec![muted],
            muted: true,
        },
    )
    .unwrap();
    (project, Files { dir, paths }, exact)
}

#[test]
fn twelve_clip_project_renders_exports_and_reopens_identically() {
    let (mut project, files, exact) = build();
    assert!(project.region_count() >= 14);
    let originals: Vec<Vec<u8>> = files
        .paths
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    let media = store_for(&project, None);
    assert_eq!(media.len(), project.media.len());
    let out = do_render(&project, &media);
    assert_eq!(
        out.report.clips, 13,
        "13 audible clips, the muted one excluded"
    );
    assert_eq!(out.report.missing_media, 0);
    // The song ends with the last region (the muted one at 13…14 s).
    assert_eq!(out.frames(), 14 * RATE as usize);

    // Left channel over the first 8 s is the exact sum of the float sines.
    let left = out.channel(0);
    let mut max_err = 0.0f64;
    for (i, &l) in left.iter().enumerate().take(8 * RATE as usize) {
        let mut expected = 0.0f64;
        for &(start, freq, amp) in &exact {
            let s0 = start * 25;
            let n = i as u64;
            if n >= s0 && n < s0 + RATE as u64 {
                expected += f64::from(
                    (amp * (2.0 * std::f64::consts::PI * freq * (n - s0) as f64 / f64::from(RATE))
                        .sin()) as f32,
                );
            }
        }
        max_err = max_err.max((f64::from(l) - expected).abs());
    }
    assert!(max_err < 1e-6, "exact clips mismatch {max_err}");
    // Stereo 16-bit clip at 8 s: balance pan 0 → unity on both channels.
    let right = out.channel(1);
    let seg = 8 * RATE as usize + 12_000..8 * RATE as usize + 36_000;
    assert!((rms(&left[seg.clone()]) - 0.2 / 2f64.sqrt()).abs() < 1e-3);
    assert!((rms(&right[seg]) - 0.1 / 2f64.sqrt()).abs() < 1e-3);
    // AIFF at 9 s on a hard-left track.
    let seg = 9 * RATE as usize + 12_000..9 * RATE as usize + 36_000;
    assert!((rms(&left[seg.clone()]) - 0.3 / 2f64.sqrt()).abs() < 1e-3);
    assert!(rms(&right[seg]) < 1e-6);
    // FLAC/MP3/OGG (44.1 kHz sources, 0.5 s each) on the stereo track.
    for i in 0..3 {
        let base = (10 + i) * RATE as usize;
        let seg = base + 4_000..base + 20_000;
        let db = |x: f64, a: f64| 20.0 * (x / (a / 2f64.sqrt())).log10();
        assert!(db(rms(&left[seg.clone()]), 0.5).abs() < 0.5, "clip {i} L");
        assert!(db(rms(&right[seg]), 0.25).abs() < 0.5, "clip {i} R");
        assert!(
            rms(&left[base + 25_000..base + 47_000]) < 1e-3,
            "clip {i} ends after 0.5 s"
        );
    }
    // The muted clip (13…14 s) is silent.
    assert!(left[13 * RATE as usize..].iter().all(|v| *v == 0.0));
    assert_eq!(out.report.tracks.len(), 5);
    assert!(out.report.tracks.iter().all(|t| t.peak > 0.0));

    // Diagnostic export: WAV + JSON report; the WAV decodes to the same samples.
    let export_path = files.dir.path().join("render.wav");
    let report_path = export(&export_path, &out).unwrap();
    let decoded = decode_file(&export_path, u64::MAX).unwrap();
    assert_eq!(decoded.planes[0], left);
    assert_eq!(decoded.planes[1], right);
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(json["clips"], 13);
    assert_eq!(json["checksum"], out.report.checksum);

    // Non-destructive edits, then save and reopen.
    let regions: Vec<RegionId> = project
        .tracks
        .iter()
        .flat_map(|t| t.regions.iter().map(|r| r.id))
        .collect();
    let apply = |p: &mut Project, c: EditCommand| elyvon_project::edit::apply(p, &c).unwrap();
    apply(
        &mut project,
        EditCommand::SplitRegions {
            ids: vec![regions[0]],
            at: SEC / 3,
        },
    );
    let (ti, ri) = project.find_region(regions[1]).unwrap();
    let r1_start = project.tracks[ti].regions[ri].start;
    apply(
        &mut project,
        EditCommand::TrimRegion {
            id: regions[1],
            start: r1_start + 200,
            end: r1_start + 1500,
        },
    );
    apply(
        &mut project,
        EditCommand::SetAudioRegion {
            id: regions[2],
            gain_db: -4.0,
            fade_in: 2000,
            fade_out: 9000,
            looped: false,
        },
    );
    apply(
        &mut project,
        EditCommand::DuplicateRegions {
            ids: vec![regions[3]],
        },
    );
    let edited = do_render(&project, &media);
    assert_ne!(edited.report.checksum, out.report.checksum);
    let path = files.dir.path().join("twelve.elyvon");
    format::save(&path, &project, "test").unwrap();
    let reopened = format::load(&path).unwrap();
    assert_eq!(reopened, project);
    let media2 = store_for(&reopened, path.parent());
    let again = do_render(&reopened, &media2);
    assert_eq!(
        again.report.checksum, edited.report.checksum,
        "same sequence after reopening"
    );
    assert_eq!(again.samples, edited.samples);
    for (p, bytes) in files.paths.iter().zip(&originals) {
        assert_eq!(
            &std::fs::read(p).unwrap(),
            bytes,
            "{} modified",
            p.display()
        );
    }
}

#[test]
fn waveform_columns_come_from_the_real_samples() {
    let (project, _files, _) = build();
    let media = store_for(&project, None);
    let map = project.transport.tempo_map().unwrap();
    let track = &project.tracks[0];
    let region = &track.regions[0];
    let elyvon_project::RegionContent::Audio {
        media: Some(id), ..
    } = region.content
    else {
        panic!()
    };
    let buf = &media[&id];
    let cols = region_columns(
        &map,
        region,
        buf,
        region.start as f64,
        region.end() as f64,
        200,
    );
    // sine0: amplitude 0.05 in every column (each column spans many cycles).
    for c in &cols {
        assert!(
            (c[0].1 - 0.05).abs() < 1e-3 && (c[0].0 + 0.05).abs() < 1e-3,
            "{c:?}"
        );
    }
    // Columns outside the region are empty; inside a trimmed region they
    // follow the source offset.
    let cols = region_columns(
        &map,
        region,
        buf,
        region.end() as f64,
        region.end() as f64 + 100.0,
        10,
    );
    assert!(cols.iter().all(|c| c[0] == (0.0, 0.0)));
    let mut trimmed = project.clone();
    elyvon_project::edit::apply(
        &mut trimmed,
        &EditCommand::TrimRegion {
            id: region.id,
            start: region.start + 960,
            end: region.end(),
        },
    )
    .unwrap();
    let r2 = &trimmed.tracks[0].regions[0];
    let elyvon_project::RegionContent::Audio { offset, .. } = r2.content else {
        panic!()
    };
    assert_eq!(offset, 24_000);
    let one = region_columns(&map, r2, buf, r2.start as f64, r2.start as f64 + 9.6, 1);
    let direct = buf.peak_range(24_000, 24_240);
    assert!((one[0][0].1 - direct[0].1).abs() < 1e-6);
}

#[test]
fn large_sessions_plan_and_zoom_within_budget() {
    // 20 minutes of 8 kHz noise-like audio: a long file for zooming.
    let rate = 8_000u32;
    let frames = 20 * 60 * rate as usize;
    let mut state = 7u32;
    let plane: Vec<f32> = (0..frames)
        .map(|i| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (state >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
            noise * (0.2 + 0.7 * ((i / 80_000) % 2) as f32)
        })
        .collect();
    let long = buffer(vec![plane.clone()], rate, rate);
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let r = s.place(t, 0, &long, "long");
    let map = s.project.transport.tempo_map().unwrap();
    let (ti, ri) = s.project.find_region(r).unwrap();
    let region = s.project.tracks[ti].regions[ri].clone();
    let span = region.end() as f64;
    let mut worst = 0.0f64;
    for (from, width) in [
        (0.0, span),
        (span / 3.0, span / 50.0),
        (span / 2.0, 1920.0),
        (span - 1000.0, 50.0),
    ] {
        let started = Instant::now();
        let cols = region_columns(&map, &region, &long, from, from + width, 2000);
        worst = worst.max(started.elapsed().as_secs_f64());
        assert_eq!(cols.len(), 2000);
        // Spot-check columns against the samples (peaks may only widen).
        for c in [0usize, 777, 1999] {
            let ta = from + width * c as f64 / 2000.0;
            let tb = from + width * (c + 1) as f64 / 2000.0;
            let fa = (map.seconds_at(ta) * f64::from(rate)).floor() as usize;
            let fb = ((map.seconds_at(tb) * f64::from(rate)).ceil() as usize)
                .min(frames)
                .max(fa + 1);
            let hi = plane[fa..fb].iter().copied().fold(f32::MIN, f32::max);
            assert!(cols[c][0].1 >= hi - 1e-4, "column {c} at zoom {width}");
        }
    }
    assert!(
        worst < 0.05,
        "a 2000-column waveform request took {worst:.3}s"
    );

    // 40 tracks × 50 regions = 2000 clips: planning stays well under a frame budget.
    let mut big = Session::new(128.0);
    let short = buffer(vec![sine(300.0, 0.1, RATE, 24_000)], RATE, RATE);
    for _ in 0..40 {
        let t = big.audio_track();
        for k in 0..50u64 {
            big.place(t, k * 700, &short, "c");
        }
    }
    let lookup = big.lookup();
    let started = Instant::now();
    let plan = build_plan(&big.project, &lookup, RATE, 1);
    let took = started.elapsed().as_secs_f64();
    assert_eq!(plan.stats.clips, 2000);
    assert!(took < 0.25, "planning 2000 clips took {took:.3}s");
}
