//! Phase 10: automation changes the real signal, sample-accurately, and the
//! offline render is identical to live playback.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use elyvon_audio::automation::{ParamOverride, StripKey};
use elyvon_audio::engine::{self, NoMidi};
use elyvon_audio::plan::build_plan;
use elyvon_audio::render::{RenderRequest, Rendered, render};
use elyvon_audio::transport::TransportCommand;
use elyvon_project::automation::{
    AutomationMode, AutomationParam, AutomationTarget, Curve, InsertParam, PointSpec,
};
use elyvon_project::automation_record::TouchPhase;
use elyvon_project::mixer::{ChannelFormat, ChannelRef, FilterMode, Processor, Send};
use elyvon_project::mixer_edit::StripPatch;
use elyvon_project::{EditCommand, TrackId};

const RATE: u32 = 48_000;

fn db(v: f64) -> f64 {
    20.0 * v.max(1e-12).log10()
}

/// A stereo track playing `seconds` of a sine (identical channels).
fn sine_track(s: &mut Session, freq: f64, amp: f64, seconds: f64) -> TrackId {
    let t = s.audio_track();
    let frames = (seconds * f64::from(RATE)) as usize;
    let x = sine(freq, amp, RATE, frames);
    s.place(t, 0, &buffer(vec![x.clone(), x], RATE, RATE), "sine");
    t
}

/// A stereo track playing a constant value.
fn dc_track(s: &mut Session, v: f32, seconds: f64) -> TrackId {
    let t = s.audio_track();
    let frames = (seconds * f64::from(RATE)) as usize;
    s.place(
        t,
        0,
        &buffer(vec![vec![v; frames], vec![v; frames]], RATE, RATE),
        "dc",
    );
    t
}

fn target(track: TrackId, param: AutomationParam) -> AutomationTarget {
    AutomationTarget {
        channel: ChannelRef::Track { id: track },
        region: None,
        param,
    }
}

fn points(s: &mut Session, t: AutomationTarget, pts: &[(u64, f64, Curve)]) {
    s.apply(EditCommand::AddAutomationPoints {
        target: t,
        points: pts
            .iter()
            .map(|&(tick, value, curve)| PointSpec {
                tick,
                value,
                curve: Some(curve),
            })
            .collect(),
    });
}

fn out(s: &Session, block: usize) -> Rendered {
    render(
        &s.project,
        &s.lookup(),
        RenderRequest {
            block_frames: block,
            ..RenderRequest::whole_song(RATE)
        },
        None,
    )
}

#[test]
fn volume_ramp_fixture_has_the_expected_rms_per_segment() {
    // 120 BPM: tick 3840 = 2 s. Linear in dB from 0 to −24 dB, then hold.
    let mut s = Session::new(120.0);
    let t = sine_track(&mut s, 1000.0, 0.5, 3.0);
    points(
        &mut s,
        target(t, AutomationParam::Volume),
        &[(0, 0.0, Curve::Linear), (3840, -24.0, Curve::Linear)],
    );
    let r = out(&s, 512);
    let left = r.channel(0);
    let seg = RATE as usize / 10; // 100 ms
    let full = 0.5 / 2f64.sqrt();
    for k in 0..28 {
        let x = &left[k * seg..(k + 1) * seg];
        let centre = (k as f64 + 0.5) * 0.1;
        let expected = db(full) + (-24.0 * centre / 2.0).max(-24.0);
        let got = db(rms(x));
        assert!(
            (got - expected).abs() < 0.05,
            "segment {k}: {got:.3} dB, expected {expected:.3} dB"
        );
    }
}

#[test]
fn automating_a_filter_cutoff_changes_the_spectrum() {
    let mut s = Session::new(120.0);
    let t = sine_track(&mut s, 5000.0, 0.5, 2.5);
    let out_id = s
        .apply(EditCommand::AddInsert {
            channel: ChannelRef::Track { id: t },
            processor: Processor::Filter {
                mode: FilterMode::LowPass,
                cutoff_hz: 20_000.0,
                q: 0.707,
            },
            index: None,
        })
        .created_inserts[0];
    let cutoff = AutomationParam::Insert {
        id: out_id,
        param: InsertParam::FilterCutoff,
    };
    points(
        &mut s,
        target(t, cutoff),
        &[(0, 200.0, Curve::Linear), (3840, 20_000.0, Curve::Linear)],
    );
    let r = out(&s, 512);
    let left = r.channel(0);
    let head = rms(&left[2400..12_000]);
    let tail = rms(&left[100_000..110_000]);
    assert!(
        db(head) < db(0.5 / 2f64.sqrt()) - 30.0,
        "5 kHz is cut at 200 Hz: {}",
        db(head)
    );
    assert!(
        (db(tail) - db(0.5 / 2f64.sqrt())).abs() < 1.0,
        "passes at 20 kHz: {}",
        db(tail)
    );
    // The sweep is monotonic in level (no zipper jumps between steps).
    let windows: Vec<f64> = (0..20)
        .map(|k| rms(&left[k * 4800..(k + 1) * 4800]))
        .collect();
    // (The first window holds the filter's start-up transient.)
    assert!(
        windows[1..].windows(2).all(|w| w[1] >= w[0] * 0.98),
        "{windows:?}"
    );
}

/// Every automatable stage at once: volume (Bézier + steps), pan, mute,
/// send level, trim, filter and insert bypass, under a tempo map.
fn busy_session() -> Session {
    let mut s = Session::new(100.0);
    s.apply(EditCommand::SetTempoChanges {
        changes: vec![
            elyvon_core::TempoChange {
                tick: 3840,
                bpm: elyvon_core::Tempo::new(150.0).unwrap(),
            },
            elyvon_core::TempoChange {
                tick: 7680,
                bpm: elyvon_core::Tempo::new(72.0).unwrap(),
            },
        ],
    });
    let t = sine_track(&mut s, 440.0, 0.4, 9.0);
    let n = dc_track(&mut s, 0.25, 9.0);
    let bus = s
        .apply(EditCommand::AddAux {
            name: Some("Verb".into()),
            sources: vec![],
        })
        .created_auxes[0];
    s.apply(EditCommand::SetSend {
        channel: ChannelRef::Track { id: t },
        target: bus,
        send: Some(Send {
            target: bus,
            level_db: -6.0,
            pre_fader: false,
            muted: false,
        }),
    });
    let trim = s
        .apply(EditCommand::AddInsert {
            channel: ChannelRef::Track { id: t },
            processor: Processor::Trim { gain_db: 0.0 },
            index: None,
        })
        .created_inserts[0];
    let filter = s
        .apply(EditCommand::AddInsert {
            channel: ChannelRef::Track { id: n },
            processor: Processor::Filter {
                mode: FilterMode::HighPass,
                cutoff_hz: 20.0,
                q: 0.9,
            },
            index: None,
        })
        .created_inserts[0];
    s.apply(EditCommand::SetStrip {
        channel: ChannelRef::Track { id: n },
        patch: StripPatch {
            format: Some(ChannelFormat::Mono),
            ..StripPatch::default()
        },
    });
    points(
        &mut s,
        target(t, AutomationParam::Volume),
        &[
            (0, -40.0, Curve::Bezier { c1: 0.0, c2: 0.2 }),
            (2000, 0.0, Curve::Step),
            (5000, -12.0, Curve::Linear),
            (9000, -3.0, Curve::Linear),
        ],
    );
    points(
        &mut s,
        target(t, AutomationParam::Pan),
        &[(0, -1.0, Curve::Linear), (12_000, 1.0, Curve::Linear)],
    );
    points(
        &mut s,
        target(t, AutomationParam::SendLevel { aux: bus }),
        &[
            (0, -96.0, Curve::Linear),
            (6000, 0.0, Curve::Step),
            (10_000, -20.0, Curve::Linear),
        ],
    );
    points(
        &mut s,
        target(
            t,
            AutomationParam::Insert {
                id: trim,
                param: InsertParam::TrimGain,
            },
        ),
        &[(0, 6.0, Curve::Linear), (14_000, -12.0, Curve::Linear)],
    );
    points(
        &mut s,
        target(n, AutomationParam::Mute),
        &[
            (0, 0.0, Curve::Step),
            (3000, 1.0, Curve::Step),
            (4500, 0.0, Curve::Step),
        ],
    );
    points(
        &mut s,
        target(
            n,
            AutomationParam::Insert {
                id: filter,
                param: InsertParam::FilterCutoff,
            },
        ),
        &[
            (0, 20.0, Curve::Linear),
            (8000, 2000.0, Curve::Bezier { c1: 1.0, c2: 1.0 }),
            (12_000, 30.0, Curve::Linear),
        ],
    );
    points(
        &mut s,
        target(
            n,
            AutomationParam::Insert {
                id: filter,
                param: InsertParam::Bypass,
            },
        ),
        &[
            (0, 0.0, Curve::Step),
            (9000, 1.0, Curve::Step),
            (11_000, 0.0, Curve::Step),
        ],
    );
    s
}

#[test]
fn renders_are_identical_for_any_block_size() {
    let s = busy_session();
    let a = out(&s, 64);
    let b = out(&s, 512);
    let c = out(&s, 1031);
    assert!(a.report.peak[0] > 0.01);
    assert_eq!(a.samples, b.samples, "64 vs 512");
    assert_eq!(a.samples, c.samples, "64 vs 1031");
}

#[test]
fn live_playback_equals_the_offline_render_including_seeks() {
    let s = busy_session();
    let offline = out(&s, 512);
    let lookup = s.lookup();
    let plan = build_plan(&s.project, &lookup, RATE, 1);
    let (mut handle, mut eng) = engine::create(RATE, 4096, plan);
    eng.set_clip_output(false);
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    // Irregular device callbacks, as a real driver might deliver them.
    let sizes = [37usize, 480, 1024, 4096, 211, 2048, 9, 1500];
    let mut live = Vec::new();
    let mut i = 0;
    while live.len() < offline.samples.len() {
        let n = sizes[i % sizes.len()];
        i += 1;
        let mut buf = vec![0.0f32; n * 2];
        eng.process(&mut buf, 2, &mut NoMidi);
        live.extend_from_slice(&buf);
    }
    live.truncate(offline.samples.len());
    if let Some(i) = live.iter().zip(&offline.samples).position(|(a, b)| a != b) {
        panic!(
            "live differs from the export at sample {i} (frame {}): {} vs {}",
            i / 2,
            live[i],
            offline.samples[i]
        );
    }

    // Locate while playing: the output continues exactly as the render
    // does from the new position (automation is a function of position).
    let tick = 6123;
    handle.send(TransportCommand::Locate { tick });
    let mut buf = vec![0.0f32; 700 * 2];
    eng.process(&mut buf, 2, &mut NoMidi);
    let map = s.project.transport.tempo_map().unwrap();
    let at = map.sample_at_tick(tick, RATE) as usize;
    // Filters keep their state across the jump, so compare a gain-only
    // track: render the volume track alone.
    let mut solo = s.project.clone();
    solo.tracks.truncate(1);
    let only = render(&solo, &lookup, RenderRequest::whole_song(RATE), None);
    let mut s2 = s.project.clone();
    s2.tracks.truncate(1);
    let plan = build_plan(&s2, &lookup, RATE, 1);
    let (mut h2, mut e2) = engine::create(RATE, 4096, plan);
    e2.set_clip_output(false);
    h2.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut warm = vec![0.0f32; 3000 * 2];
    e2.process(&mut warm, 2, &mut NoMidi);
    h2.send(TransportCommand::Locate { tick });
    let mut after = vec![0.0f32; 700 * 2];
    e2.process(&mut after, 2, &mut NoMidi);
    // The aux bus has no state either; the first frames match exactly.
    assert_eq!(&after[..], &only.samples[at * 2..(at + 700) * 2]);
}

#[test]
fn thousands_of_points_with_abrupt_changes_stay_click_free_and_land_on_their_frames() {
    let mut s = Session::new(97.0);
    s.apply(EditCommand::SetTempoChanges {
        changes: (1..12)
            .map(|k| elyvon_core::TempoChange {
                tick: k * 1777,
                bpm: elyvon_core::Tempo::new(60.0 + 13.0 * k as f64).unwrap(),
            })
            .collect(),
    });
    let t = dc_track(&mut s, 0.5, 12.0);
    // 2 000 steps between silence and unity, pseudo-random spacing ≥ 240 frames.
    let mut seed = 0x1234_5678u64;
    let mut tick = 0u64;
    let mut pts = Vec::new();
    for k in 0..2000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        tick += 11 + (seed >> 59);
        pts.push((tick, if k % 2 == 0 { -96.0 } else { 0.0 }, Curve::Step));
    }
    points(&mut s, target(t, AutomationParam::Volume), &pts);
    let started = std::time::Instant::now();
    let r = out(&s, 512);
    let elapsed = started.elapsed();
    let left = r.channel(0);
    // The clip itself ends at 12 s; look at the automated part.
    let (worst, max_step) = left[..left.len() - 64]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .enumerate()
        .fold((0, 0.0f32), |a, (i, d)| if d > a.1 { (i, d) } else { a });
    eprintln!("largest step {max_step} at frame {worst} of {}", left.len());
    // 0.5 over the 96-frame de-click ramp, plus float rounding.
    assert!(
        max_step <= 0.5 / 96.0 + 1e-4,
        "largest sample step {max_step}"
    );
    let map = s.project.transport.tempo_map().unwrap();
    for &(tick, value, _) in pts.iter().step_by(97) {
        let f = map.sample_at_tick(tick, RATE) as usize;
        if f + 1 >= left.len() {
            break;
        }
        // The point's own frame carries its value (after the ramp).
        let expected = if value == 0.0 { 0.5 } else { 0.0 };
        assert!(
            (left[f] - expected).abs() < 1e-6,
            "tick {tick} frame {f}: {}",
            left[f]
        );
    }
    eprintln!("2000-point lane, 12 s render with 11 tempo changes: {elapsed:?}");
}

#[test]
fn a_hundred_thousand_points_compile_and_evaluate_quickly() {
    let mut s = Session::new(120.0);
    let t = dc_track(&mut s, 0.5, 60.0);
    let pts: Vec<(u64, f64, Curve)> = (0..100_000u64)
        .map(|k| {
            (
                k * 2,
                -48.0 + 48.0 * ((k as f64) * 0.01).sin().abs(),
                Curve::Linear,
            )
        })
        .collect();
    points(&mut s, target(t, AutomationParam::Volume), &pts);
    let lookup = s.lookup();
    let started = std::time::Instant::now();
    let plan = build_plan(&s.project, &lookup, RATE, 1);
    let compile = started.elapsed();
    assert_eq!(plan.stats.envelopes, 1);
    let started = std::time::Instant::now();
    let r = render(
        &s.project,
        &lookup,
        RenderRequest {
            end_tick: Some(200_000),
            ..RenderRequest::whole_song(RATE)
        },
        None,
    );
    let render_time = started.elapsed();
    assert!(r.report.peak[0] > 0.4);
    eprintln!(
        "100 000 points: plan {compile:?}, {:.1} s render {render_time:?}",
        r.report.seconds
    );
    assert!(compile.as_secs_f64() < 2.0, "plan build {compile:?}");
}

#[test]
fn off_mode_ignores_lanes_and_read_mode_plays_them() {
    let mut s = Session::new(120.0);
    let t = dc_track(&mut s, 0.5, 2.0);
    points(
        &mut s,
        target(t, AutomationParam::Volume),
        &[(0, -96.0, Curve::Linear), (100, -96.0, Curve::Linear)],
    );
    assert!(out(&s, 512).report.peak[0] < 1e-6, "read: silent");
    s.apply(EditCommand::SetAutomationMode {
        channel: ChannelRef::Track { id: t },
        mode: AutomationMode::Off,
    });
    assert!(
        (out(&s, 512).report.peak[0] - 0.5).abs() < 1e-6,
        "off: static fader"
    );
}

#[test]
fn region_lanes_override_the_track_lane_inside_their_region_and_move_with_it() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let b = buffer(
        vec![vec![0.5; RATE as usize], vec![0.5; RATE as usize]],
        RATE,
        RATE,
    );
    let r1 = s.place(t, 0, &b, "a");
    let r2 = s.place(t, 3840, &b, "b");
    points(
        &mut s,
        target(t, AutomationParam::Volume),
        &[(0, -6.0, Curve::Linear)],
    );
    s.apply(EditCommand::AddAutomationPoints {
        target: AutomationTarget {
            channel: ChannelRef::Track { id: t },
            region: Some(r2),
            param: AutomationParam::Volume,
        },
        points: vec![PointSpec {
            tick: 0,
            value: -20.0,
            curve: None,
        }],
    });
    let r = out(&s, 512);
    let g = |frame: usize| f64::from(r.samples[frame * 2]);
    assert!(
        (db(g(10_000)) - (db(0.5) - 6.0)).abs() < 1e-3,
        "track lane in region 1"
    );
    assert!(
        (db(g(96_000 + 10_000)) - (db(0.5) - 20.0)).abs() < 1e-3,
        "region lane in region 2"
    );
    let _ = r1;
    s.apply(EditCommand::MoveRegions {
        ids: vec![r2],
        delta_ticks: 3840,
        track_offset: 0,
    });
    let r = out(&s, 512);
    assert!((db(f64::from(r.samples[(192_000 + 10_000) * 2])) - (db(0.5) - 20.0)).abs() < 1e-3);
}

#[test]
fn touch_overrides_reach_the_callback_without_clicks_and_release_to_the_lane() {
    let mut s = Session::new(120.0);
    let t = dc_track(&mut s, 0.5, 6.0);
    points(
        &mut s,
        target(t, AutomationParam::Volume),
        &[(0, -6.0, Curve::Linear)],
    );
    s.apply(EditCommand::SetAutomationMode {
        channel: ChannelRef::Track { id: t },
        mode: AutomationMode::Touch,
    });
    let lookup = s.lookup();
    let plan = build_plan(&s.project, &lookup, RATE, 1);
    let (mut handle, mut eng) = engine::create(RATE, 1024, plan);
    eng.set_clip_output(false);
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut all = Vec::new();
    let run = |eng: &mut engine::Engine, blocks: usize, all: &mut Vec<f32>| {
        for _ in 0..blocks {
            let mut buf = vec![0.0f32; 512 * 2];
            eng.process(&mut buf, 2, &mut NoMidi);
            all.extend_from_slice(&buf);
        }
    };
    run(&mut eng, 20, &mut all);
    let key = StripKey::Track(t.0.as_u128());
    let send = |h: &mut engine::EngineHandle, value: f64, phase: TouchPhase| {
        assert!(h.send_override(ParamOverride {
            strip: key,
            param: AutomationParam::Volume,
            value,
            phase,
        }));
    };
    send(&mut handle, 0.0, TouchPhase::Begin);
    run(&mut eng, 20, &mut all);
    send(&mut handle, 0.0, TouchPhase::End);
    run(&mut eng, 40, &mut all);
    let left: Vec<f32> = all.iter().step_by(2).copied().collect();
    let lane = 0.5 * 10f32.powf(-6.0 / 20.0);
    assert!(
        (left[5000] - lane).abs() < 1e-5,
        "reads the lane before the touch"
    );
    assert!((left[512 * 30] - 0.5).abs() < 1e-5, "touched value (0 dB)");
    assert!(
        (left[512 * 59] - lane).abs() < 1e-5,
        "released back to the lane"
    );
    let max_step = left
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step < 0.001, "no click: {max_step}");
    // Stop clears gestures.
    send(&mut handle, -30.0, TouchPhase::Begin);
    handle.send(TransportCommand::Stop);
    let mut buf = vec![0.0f32; 512 * 2];
    eng.process(&mut buf, 2, &mut NoMidi);
}
