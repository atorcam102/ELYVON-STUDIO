//! Deterministic playback tests through the offline renderer (same engine
//! code as the device callback).

// Test helpers outside `#[test]` functions may panic on failure.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use elyvon_audio::render::{RenderRequest, Rendered, render};
use elyvon_project::{EditCommand, RegionContent};

const RATE: u32 = 48_000;
/// At 120 BPM and 48 kHz one tick is exactly 25 frames.
const FRAMES_PER_TICK: u64 = 25;

fn render_session(s: &Session, block: usize, end_tick: Option<u64>) -> Rendered {
    let lookup = s.lookup();
    render(
        &s.project,
        &lookup,
        RenderRequest {
            block_frames: block,
            end_tick,
            ..RenderRequest::whole_song(RATE)
        },
        None,
    )
}

fn set_audio(
    s: &mut Session,
    id: elyvon_project::RegionId,
    gain_db: f64,
    fade_in: u64,
    fade_out: u64,
    looped: bool,
) {
    s.apply(EditCommand::SetAudioRegion {
        id,
        gain_db,
        fade_in,
        fade_out,
        looped,
    });
}

#[test]
fn attacks_land_on_the_exact_frame_for_any_block_size() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let mut impulse = vec![0.0f32; 50];
    impulse[0] = 1.0;
    let imp = buffer(vec![impulse], RATE, RATE);
    let starts = [10u64, 11, 20, 21, 100, 513];
    for &tick in &starts {
        s.place(t, tick, &imp, "imp");
    }
    // Hard-left pan makes the left channel exactly the source.
    s.apply(EditCommand::SetTrackPan { id: t, pan: -1.0 });
    let reference = render_session(&s, 512, Some(600));
    for block in [1, 24, 25, 26, 127, 512, 4096] {
        let r = render_session(&s, block, Some(600));
        assert_eq!(
            r.report.checksum, reference.report.checksum,
            "block {block}"
        );
        let left = r.channel(0);
        let hits: Vec<usize> = left
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0.0)
            .map(|(i, _)| i)
            .collect();
        let expected: Vec<usize> = starts
            .iter()
            .map(|t| (t * FRAMES_PER_TICK) as usize)
            .collect();
        assert_eq!(hits, expected, "block {block}");
        assert!(hits.iter().all(|&i| left[i] == 1.0));
    }
}

#[test]
fn mono_uses_constant_power_pan_and_stereo_uses_balance() {
    let mut s = Session::new(120.0);
    let mono_track = s.audio_track();
    let stereo_track = s.audio_track();
    let mono = buffer(vec![vec![1.0; 1000]], RATE, RATE);
    let stereo = buffer(vec![vec![1.0; 1000], vec![1.0; 1000]], RATE, RATE);
    s.place(mono_track, 0, &mono, "mono");
    s.place(stereo_track, 100, &stereo, "stereo");
    let check = |s: &Session, frame: usize, l: f32, r: f32| {
        let out = render_session(s, 256, Some(140));
        assert!(
            (out.samples[frame * 2] - l).abs() < 1e-6,
            "L {} != {l}",
            out.samples[frame * 2]
        );
        assert!(
            (out.samples[frame * 2 + 1] - r).abs() < 1e-6,
            "R {} != {r}",
            out.samples[frame * 2 + 1]
        );
    };
    let h = std::f32::consts::FRAC_1_SQRT_2;
    check(&s, 10, h, h);
    check(&s, 2600, 1.0, 1.0);
    s.apply(EditCommand::SetTrackPan {
        id: mono_track,
        pan: -1.0,
    });
    s.apply(EditCommand::SetTrackPan {
        id: stereo_track,
        pan: 0.5,
    });
    check(&s, 10, 1.0, 0.0);
    check(&s, 2600, 0.5, 1.0);
    s.apply(EditCommand::SetTrackVolume {
        id: stereo_track,
        volume_db: -6.0,
    });
    let g = 10f32.powf(-6.0 / 20.0);
    check(&s, 2600, 0.5 * g, g);
}

#[test]
fn later_region_wins_and_covered_region_resumes() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let a = buffer(vec![vec![0.5; 10_000]], RATE, RATE);
    let b = buffer(vec![vec![0.25; 2_500]], RATE, RATE);
    s.apply(EditCommand::SetTrackPan { id: t, pan: -1.0 });
    let ra = s.place(t, 0, &a, "a"); // frames 0..10000
    let rb = s.place(t, 100, &b, "b"); // frames 2500..5000
    let out = render_session(&s, 333, Some(400)).channel(0);
    assert!(out[..2500].iter().all(|&v| v == 0.5));
    assert!(out[2500..5000].iter().all(|&v| v == 0.25));
    assert!(out[5000..10_000].iter().all(|&v| v == 0.5));
    assert!(out[10_000..].iter().all(|&v| v == 0.0));

    // A fade-in on the covering region crossfades (equal power) with the
    // covered one; a fade-out crossfades back.
    set_audio(&mut s, rb, 0.0, 400, 200, false);
    let out = render_session(&s, 333, Some(400)).channel(0);
    let q = |x: f32| (x * std::f32::consts::FRAC_PI_2).sin();
    for d in [0usize, 1, 100, 399] {
        let x = d as f32 / 400.0;
        let expected = 0.5 * q(1.0 - x) + 0.25 * q(x);
        assert!((out[2500 + d] - expected).abs() < 1e-6, "xfade in at {d}");
    }
    assert_eq!(out[2900], 0.25);
    for d in [0usize, 50, 199] {
        // Fade-out of b over [4800, 5000): remaining = 200 - d.
        let rem = (200 - d) as f32 / 200.0;
        let expected = 0.25 * q(rem) + 0.5 * q(1.0 - rem);
        assert!((out[4800 + d] - expected).abs() < 1e-6, "xfade out at {d}");
    }
    // The covered region's own timing is untouched.
    let _ = ra;
}

#[test]
fn looped_regions_repeat_the_source_end_to_start() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    s.apply(EditCommand::SetTrackPan { id: t, pan: -1.0 });
    let ramp: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
    let r = s.place(t, 0, &buffer(vec![ramp], RATE, RATE), "ramp");
    set_audio(&mut s, r, 0.0, 0, 0, true);
    s.apply(EditCommand::TrimRegion {
        id: r,
        start: 0,
        end: 14, // 350 frames
    });
    let out = render_session(&s, 64, Some(20)).channel(0);
    for (i, &v) in out.iter().enumerate().take(350) {
        assert_eq!(v, (i % 100) as f32 / 100.0, "frame {i}");
    }
    assert!(out[350..].iter().all(|&v| v == 0.0));
    // Non-destructive trim of the start: the timeline keeps playing the same
    // source frames.
    s.apply(EditCommand::TrimRegion {
        id: r,
        start: 2,
        end: 14,
    });
    let out2 = render_session(&s, 64, Some(20)).channel(0);
    assert!(out2[..50].iter().all(|&v| v == 0.0));
    assert_eq!(&out2[50..350], &out[50..350]);
}

#[test]
fn clip_fades_gain_and_region_mute() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    s.apply(EditCommand::SetTrackPan { id: t, pan: -1.0 });
    let r = s.place(t, 0, &buffer(vec![vec![1.0; 1000]], RATE, RATE), "dc");
    set_audio(&mut s, r, -6.0, 100, 100, false);
    let g = 10f32.powf(-6.0 / 20.0);
    let out = render_session(&s, 100, Some(40)).channel(0);
    assert_eq!(out[0], 0.0);
    assert!((out[50] - g * (0.5f32 * std::f32::consts::FRAC_PI_2).sin()).abs() < 1e-6);
    assert!((out[500] - g).abs() < 1e-6);
    assert!((out[999] - g * (0.01f32 * std::f32::consts::FRAC_PI_2).sin()).abs() < 1e-6);
    s.apply(EditCommand::SetRegionsMuted {
        ids: vec![r],
        muted: true,
    });
    let out = render_session(&s, 100, Some(40));
    assert!(out.samples.iter().all(|&v| v == 0.0));
    assert_eq!(out.report.clips, 0);
}

#[test]
fn track_mute_and_solo_follow_project_rules() {
    let mut s = Session::new(120.0);
    let a = s.audio_track();
    let b = s.audio_track();
    let one = buffer(vec![vec![0.5; 2000]], RATE, RATE);
    s.place(a, 0, &one, "a");
    s.place(b, 0, &one, "b");
    let level = |s: &Session| render_session(s, 128, Some(40)).samples[100];
    let both = level(&s);
    s.apply(EditCommand::SetTracksSoloed {
        ids: vec![a],
        soloed: true,
    });
    assert!((level(&s) - both / 2.0).abs() < 1e-6);
    s.apply(EditCommand::SetTracksMuted {
        ids: vec![a],
        muted: true,
    });
    assert_eq!(level(&s), 0.0, "mute wins over solo and b is not soloed");
}

#[test]
fn missing_media_is_silent_and_reported() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let buf = buffer(vec![vec![0.5; 2000]], RATE, RATE);
    let r = s.place(t, 0, &buf, "x");
    let (ti, ri) = s.project.find_region(r).unwrap();
    let RegionContent::Audio {
        media: Some(id), ..
    } = s.project.tracks[ti].regions[ri].content
    else {
        panic!()
    };
    s.media.remove(&id);
    let out = render_session(&s, 128, Some(80));
    assert!(out.samples.iter().all(|&v| v == 0.0));
    assert_eq!(out.report.missing_media, 1);
    assert_eq!(out.report.clips, 0);
}

#[test]
fn different_source_rates_play_at_the_right_speed_and_level() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    s.apply(EditCommand::SetTrackPan { id: t, pan: -1.0 });
    // 1 s of 1 kHz at 44.1 kHz → 48 000 engine frames.
    let src = buffer(vec![sine(1000.0, 0.5, 44_100, 44_100)], 44_100, RATE);
    assert_eq!(src.frames(), 48_000);
    let r = s.place(t, 0, &src, "44k");
    let (ti, ri) = s.project.find_region(r).unwrap();
    assert_eq!(
        s.project.tracks[ti].regions[ri].length, 1920,
        "1 s = 2 beats"
    );
    let out = render_session(&s, 512, None).channel(0);
    assert_eq!(out.len(), 48_000);
    let mid = &out[12_000..36_000];
    assert!((rms(mid) - 0.5 / 2f64.sqrt()).abs() < 0.005);
    // Zero crossings ≈ 2 per cycle: 1000 Hz in 0.5 s → ~1000 crossings.
    let crossings = mid
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    assert!((crossings as i64 - 1000).abs() <= 2, "{crossings}");
}
