//! Phase 11: non-destructive editing changes what plays, never the source
//! files, and every result is verified numerically on the rendered audio.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use common::*;
use elyvon_audio::analysis;
use elyvon_audio::media::MediaBuffer;
use elyvon_audio::render::{RenderRequest, Rendered, render};
use elyvon_project::mixer::{ChannelRef, FilterMode, Processor};
use elyvon_project::{
    EditCommand, FadeCurve, MediaAsset, MediaId, RegionContent, RegionId, TrackId,
};

const RATE: u32 = 48_000;

fn out(s: &Session) -> Rendered {
    render(
        &s.project,
        &s.lookup(),
        RenderRequest::whole_song(RATE),
        None,
    )
}

fn stereo(x: Vec<f32>) -> Arc<MediaBuffer> {
    buffer(vec![x.clone(), x], RATE, RATE)
}

/// Deterministic white noise in ±amp.
fn noise(seed: u64, amp: f32, n: usize) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            amp * (((s >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0)
        })
        .collect()
}

fn audio(s: &Session, id: RegionId) -> (u64, u64, u64, u64, bool) {
    let (t, r) = s.project.find_region(id).unwrap();
    let region = &s.project.tracks[t].regions[r];
    match region.content {
        RegionContent::Audio {
            offset,
            fade_in,
            reversed,
            ..
        } => (region.start, region.length, offset, fade_in, reversed),
        _ => panic!("audio"),
    }
}

/// Two regions on one track, touching at 1 s (120 BPM: tick 1920), each
/// cut from the middle of its own file so there is audio beyond the cut.
fn touching(s: &mut Session, a: Vec<f32>, b: Vec<f32>) -> (TrackId, RegionId, RegionId) {
    let t = s.audio_track();
    let ra = s.place(t, 0, &stereo(a), "a");
    let rb = s.place(t, 0, &stereo(b), "b");
    // a: [0, 1 s) from source 0; b: [1 s, 2 s) from source 1 s.
    s.apply(EditCommand::TrimRegion {
        id: ra,
        start: 0,
        end: 1920,
    });
    s.apply(EditCommand::TrimRegion {
        id: rb,
        start: 1920,
        end: 3840,
    });
    (t, ra, rb)
}

#[test]
fn crossfades_sum_correctly_for_every_fade_shape() {
    // Identical DC: linear and S-curve crossfades are constant gain.
    for curve in [FadeCurve::Linear, FadeCurve::SCurve] {
        let mut s = Session::new(120.0);
        let (_, ra, rb) = touching(&mut s, vec![0.5; 96_000], vec![0.5; 96_000]);
        s.apply(EditCommand::SetFadeCurve {
            ids: vec![ra, rb],
            curve,
        });
        s.apply(EditCommand::CreateCrossfade {
            left: ra,
            right: rb,
            length_ms: 40.0,
        });
        let (bs, _, _, fade, _) = audio(&s, rb);
        // 40 ms rounded to whole ticks on each side of the cut.
        assert!(
            bs < 1920 && fade >= 1880,
            "centred overlap: start {bs}, fade {fade}"
        );
        let r = out(&s);
        let left = r.channel(0);
        let worst = left[1_000..90_000]
            .iter()
            .map(|v| (v - 0.5).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-5, "{curve:?}: deviation {worst}");
    }
    // Uncorrelated noise: equal power keeps the level, linear dips ~3 dB.
    let a = noise(1, 0.5, 96_000);
    let b = noise(2, 0.5, 96_000);
    let level = |curve: FadeCurve| -> f64 {
        let mut s = Session::new(120.0);
        let (_, ra, rb) = touching(&mut s, a.clone(), b.clone());
        s.apply(EditCommand::SetFadeCurve {
            ids: vec![ra, rb],
            curve,
        });
        s.apply(EditCommand::CreateCrossfade {
            left: ra,
            right: rb,
            length_ms: 200.0,
        });
        let left = out(&s).channel(0);
        // RMS in the middle 50 ms of the crossfade vs outside it.
        rms(&left[46_800..49_200]) / rms(&left[20_000..40_000])
    };
    let ep = 20.0 * level(FadeCurve::EqualPower).log10();
    let lin = 20.0 * level(FadeCurve::Linear).log10();
    assert!(ep.abs() < 0.6, "equal power keeps the level: {ep:.2} dB");
    assert!(
        (lin + 3.0).abs() < 0.8,
        "linear dips about 3 dB: {lin:.2} dB"
    );
}

#[test]
fn reverse_plays_backwards_and_splitting_a_reversed_region_changes_nothing() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let ramp: Vec<f32> = (0..48_000).map(|i| i as f32 / 48_000.0).collect();
    let r = s.place(t, 0, &stereo(ramp.clone()), "ramp");
    let src_before = s.project.clone();
    s.apply(EditCommand::SetRegionsReversed {
        ids: vec![r],
        reversed: true,
    });
    let left = out(&s).channel(0);
    for i in [0usize, 1000, 30_000, 47_999] {
        assert!((left[i] - ramp[47_999 - i]).abs() < 1e-6, "frame {i}");
    }
    let whole = out(&s).samples;
    s.apply(EditCommand::SplitRegions {
        ids: vec![r],
        at: 640,
    });
    let split = out(&s).samples;
    assert_eq!(split, whole, "split reversed region renders identically");
    // Trimming the end of a reversed region keeps the rest in place.
    s.apply(EditCommand::TrimRegion {
        id: r,
        start: 0,
        end: 320,
    });
    let trimmed = out(&s).channel(0);
    assert!((trimmed[100] - whole[200]).abs() < 1e-6);
    // Looping and reversing are exclusive.
    assert!(
        elyvon_project::edit::apply(
            &mut s.project.clone(),
            &EditCommand::SetAudioRegion {
                id: r,
                gain_db: 0.0,
                fade_in: 0,
                fade_out: 0,
                looped: true
            }
        )
        .is_err()
    );
    // The media pool entry is unchanged (only the region references differ).
    assert_eq!(s.project.media, src_before.media);
}

#[test]
fn normalize_sets_a_non_destructive_gain_that_reaches_the_target_peak() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let x = sine(440.0, 0.2, RATE, 48_000);
    let r = s.place(t, 0, &stereo(x), "quiet");
    let map = s.project.transport.tempo_map().unwrap();
    let (ti, ri) = s.project.find_region(r).unwrap();
    let region = s.project.tracks[ti].regions[ri].clone();
    let RegionContent::Audio {
        media: Some(id), ..
    } = region.content
    else {
        panic!()
    };
    let peak = analysis::region_peak(&map, &region, &s.media[&id]).unwrap();
    let gain = analysis::normalize_gain(peak, -1.0).unwrap();
    s.apply(EditCommand::NormalizeRegions {
        gains: vec![elyvon_project::region_edit::RegionGain {
            id: r,
            gain_db: gain,
        }],
    });
    let got = 20.0 * f64::from(out(&s).report.peak[0]).log10();
    assert!((got + 1.0).abs() < 0.01, "peak {got:.4} dBFS");
}

#[test]
fn transients_are_found_where_hits_start() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let mut x = noise(7, 0.001, 96_000);
    let hits = [4_800usize, 20_000, 33_333, 60_000, 81_000];
    for &h in &hits {
        for (k, v) in noise(h as u64, 0.8, 4_000).into_iter().enumerate() {
            x[h + k] += v * (-(k as f32) / 600.0).exp();
        }
    }
    let r = s.place(t, 0, &stereo(x), "drums");
    let map = s.project.transport.tempo_map().unwrap();
    let (ti, ri) = s.project.find_region(r).unwrap();
    let region = s.project.tracks[ti].regions[ri].clone();
    let RegionContent::Audio {
        media: Some(id), ..
    } = region.content
    else {
        panic!()
    };
    let ticks = analysis::detect_transients(&map, &region, &s.media[&id], 0.5);
    assert_eq!(ticks.len(), hits.len(), "{ticks:?}");
    for (tick, hit) in ticks.iter().zip(hits) {
        let frame = map.sample_at_tick(*tick, RATE) as i64;
        assert!((frame - hit as i64).abs() <= 48, "onset {frame} vs {hit}");
    }
    // Slices at the transients keep the sound identical.
    let before = out(&s).samples;
    let outcome = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::SliceRegion {
            id: r,
            positions: ticks.clone(),
        },
    )
    .unwrap();
    assert_eq!(outcome.created_regions.len(), hits.len());
    let after = out(&s).samples;
    let worst = before
        .iter()
        .zip(&after)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-6, "slicing is inaudible: {worst}");
}

#[test]
fn slip_moves_the_audio_inside_the_region() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let ramp: Vec<f32> = (0..96_000).map(|i| i as f32 / 96_000.0).collect();
    let r = s.place(t, 0, &stereo(ramp.clone()), "ramp");
    s.apply(EditCommand::TrimRegion {
        id: r,
        start: 960,
        end: 2880,
    });
    let before = out(&s).channel(0);
    // Content 480 ticks (0.25 s) later: what played at t now plays at t + 0.25 s.
    s.apply(EditCommand::SlipRegions {
        ids: vec![r],
        delta_ticks: 480,
    });
    let (start, length, offset, _, _) = audio(&s, r);
    assert_eq!((start, length), (960, 1920), "the region stays put");
    assert_eq!(offset, 24_000 - 12_000);
    let after = out(&s).channel(0);
    assert!((after[24_000 + 12_000 + 5] - before[24_000 + 5]).abs() < 1e-6);
}

#[test]
fn glue_joins_contiguous_pieces_and_refuses_unrelated_audio() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let x = sine(220.0, 0.5, RATE, 96_000);
    let r = s.place(t, 0, &stereo(x), "x");
    let whole = out(&s).samples;
    let right = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::SplitRegions {
            ids: vec![r],
            at: 1000,
        },
    )
    .unwrap()
    .created_regions[0];
    let outcome = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::GlueRegions {
            ids: vec![r, right],
        },
    )
    .unwrap();
    assert_eq!(s.project.tracks[0].regions.len(), 1);
    assert_eq!(outcome.selected_regions, vec![r]);
    assert_eq!(out(&s).samples, whole);
    let other = s.place(t, 3840, &stereo(vec![0.1; 9600]), "other");
    assert!(
        elyvon_project::edit::apply(
            &mut s.project.clone(),
            &EditCommand::GlueRegions {
                ids: vec![r, other]
            }
        )
        .is_err()
    );
}

#[test]
fn ripple_delete_and_marquee_ranges_close_or_keep_gaps() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let b = stereo(vec![0.25; 48_000]);
    let r1 = s.place(t, 0, &b, "1");
    let r2 = s.place(t, 1920, &b, "2");
    let r3 = s.place(t, 5760, &b, "3");
    s.apply(EditCommand::RippleDelete { ids: vec![r2] });
    assert_eq!(audio(&s, r3).0, 5760 - 1920, "later material moves left");
    assert_eq!(audio(&s, r1).0, 0);
    // Marquee split selects exactly the inside part.
    let o = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::SplitRange {
            track_ids: vec![t],
            start: 960,
            end: 1440,
        },
    )
    .unwrap();
    assert_eq!(o.selected_regions.len(), 1);
    let inside = o.selected_regions[0];
    assert_eq!((audio(&s, inside).0, audio(&s, inside).1), (960, 480));
    // Marquee delete without ripple leaves silence; with ripple closes it.
    let before_end = s.project.content_end();
    s.apply(EditCommand::CutRange {
        track_ids: vec![t],
        start: 960,
        end: 1440,
        ripple: false,
    });
    assert_eq!(s.project.content_end(), before_end);
    let left = out(&s).channel(0);
    assert!(left[30_000].abs() < 1e-9 && (left[10_000] - 0.25).abs() < 1e-6);
    s.apply(EditCommand::CutRange {
        track_ids: vec![t],
        start: 0,
        end: 480,
        ripple: true,
    });
    assert_eq!(s.project.content_end(), before_end - 480);
}

#[test]
fn phase_locked_group_edits_keep_multitrack_recordings_aligned() {
    let mut s = Session::new(120.0);
    let kick = s.audio_track();
    let snare = s.audio_track();
    let b = stereo(sine(100.0, 0.5, RATE, 96_000));
    let rk = s.place(kick, 0, &b, "kick");
    let rs = s.place(snare, 0, &b, "snare");
    let group = s
        .apply(EditCommand::AddGroup {
            name: Some("Drums".into()),
            members: vec![
                ChannelRef::Track { id: kick },
                ChannelRef::Track { id: snare },
            ],
        })
        .created_groups[0];
    let mut g = s.project.mixer.group(group).unwrap().clone();
    g.link_edit = true;
    s.apply(EditCommand::UpdateGroup { group: g });
    s.apply(EditCommand::SplitRegions {
        ids: vec![rk],
        at: 960,
    });
    assert_eq!(
        s.project.tracks[1].regions.len(),
        2,
        "the snare was split too"
    );
    s.apply(EditCommand::SlipRegions {
        ids: vec![rk],
        delta_ticks: -48,
    });
    s.apply(EditCommand::MoveRegions {
        ids: vec![rk],
        delta_ticks: 96,
        track_offset: 0,
    });
    s.apply(EditCommand::TrimRegion {
        id: rk,
        start: 200,
        end: 900,
    });
    assert_eq!(
        audio(&s, rk),
        audio(&s, rs),
        "start, length and source offset match"
    );
}

#[test]
fn freeze_plays_the_rendered_file_identically_and_refuses_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let x = noise(3, 0.7, 96_000);
    s.place(t, 0, &buffer(vec![x], RATE, RATE), "mono");
    for p in [
        Processor::Filter {
            mode: FilterMode::LowPass,
            cutoff_hz: 2000.0,
            q: 0.8,
        },
        Processor::Limiter {
            ceiling_db: -6.0,
            lookahead_ms: 5.0,
            release_ms: 60.0,
        },
    ] {
        s.apply(EditCommand::AddInsert {
            channel: ChannelRef::Track { id: t },
            processor: p,
            index: None,
        });
    }
    s.apply(EditCommand::SetTrackPan { id: t, pan: 0.4 });
    s.apply(EditCommand::SetTrackVolume {
        id: t,
        volume_db: -3.0,
    });
    let live = out(&s);
    let latency = elyvon_audio::plan::build_plan(&s.project, &s.lookup(), RATE, 1)
        .stats
        .latency_frames;
    assert_eq!(latency, 240, "5 ms look-ahead");
    let r = analysis::render_track(&s.project, &s.lookup(), t, None, false, 1.0, RATE).unwrap();
    let path = dir.path().join("Freeze").join("t.wav");
    analysis::write_render(&path, &r).unwrap();
    let buf = elyvon_audio::media::MediaStore::load_file(&path, RATE, 1 << 30).unwrap();
    let id = MediaId::new();
    s.media.insert(id, Arc::clone(&buf));
    s.apply(EditCommand::FreezeTrack {
        id: t,
        media: MediaAsset {
            id,
            name: "freeze".into(),
            path: path.display().to_string(),
            info: Some(buf.info.clone()),
        },
        start: r.start,
    });
    let plan = elyvon_audio::plan::build_plan(&s.project, &s.lookup(), RATE, 1);
    assert_eq!(
        (
            plan.stats.frozen,
            plan.stats.stale_freezes,
            plan.stats.inserts
        ),
        (1, 0, 0)
    );
    let frozen = out(&s);
    // Live playback is delayed by the limiter look-ahead; the freeze file is
    // aligned with the timeline (and the frozen track has no latency left).
    let live = &live.samples[latency * 2..];
    let n = live.len().min(frozen.samples.len());
    let worst = live[..n]
        .iter()
        .zip(&frozen.samples[..n])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-5, "frozen playback matches live: {worst}");
    // Editing the frozen material is refused; fader moves are fine.
    let insert = s.project.tracks[0].strip.inserts[0].id;
    assert!(
        elyvon_project::edit::apply(
            &mut s.project.clone(),
            &EditCommand::RemoveInsert {
                channel: ChannelRef::Track { id: t },
                id: insert
            }
        )
        .is_err()
    );
    let region = s.project.tracks[0].regions[0].id;
    assert!(
        elyvon_project::edit::apply(
            &mut s.project.clone(),
            &EditCommand::SplitRegions {
                ids: vec![region],
                at: 960
            }
        )
        .is_err()
    );
    s.apply(EditCommand::SetTrackVolume {
        id: t,
        volume_db: -6.0,
    });
    // A tempo change makes the freeze stale: the track plays live again.
    s.apply(EditCommand::SetTempo { bpm: 100.0 });
    let plan = elyvon_audio::plan::build_plan(&s.project, &s.lookup(), RATE, 1);
    assert_eq!((plan.stats.frozen, plan.stats.stale_freezes), (0, 1));
    s.apply(EditCommand::UnfreezeTrack { id: t });
    assert!(s.project.tracks[0].freeze.is_none());
}

#[test]
fn bounce_in_place_renders_through_the_inserts_and_mutes_the_sources() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let r = s.place(t, 960, &stereo(sine(330.0, 0.6, RATE, 48_000)), "x");
    s.apply(EditCommand::AddInsert {
        channel: ChannelRef::Track { id: t },
        processor: Processor::Trim { gain_db: -6.0 },
        index: None,
    });
    let before = out(&s);
    let b =
        analysis::render_track(&s.project, &s.lookup(), t, Some(&[r]), true, 0.5, RATE).unwrap();
    let path = dir.path().join("Bounces").join("x.wav");
    analysis::write_render(&path, &b).unwrap();
    let buf = elyvon_audio::media::MediaStore::load_file(&path, RATE, 1 << 30).unwrap();
    let id = MediaId::new();
    s.media.insert(id, Arc::clone(&buf));
    let o = s.apply(EditCommand::BounceInPlace {
        asset: MediaAsset {
            id,
            name: "x bounce".into(),
            path: path.display().to_string(),
            info: Some(buf.info.clone()),
        },
        start: b.start,
        source_track: t,
        regions: vec![r],
        mute_sources: true,
    });
    assert_eq!(o.created_tracks.len(), 1);
    assert!(s.project.tracks[0].regions[0].muted);
    let after = out(&s);
    let n = before.samples.len().min(after.samples.len());
    let worst = before.samples[..n]
        .iter()
        .zip(&after.samples[..n])
        .map(|(a, c)| (a - c).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-5, "the bounce sounds like the original: {worst}");
}
