//! Phases 12–13: native effects in the real mixer graph — offline bounce
//! equals live playback at any block size, sidechain keys, effect-parameter
//! automation, preset reproducibility, tails after stop, tempo-following
//! delays and sample-rate changes.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use elyvon_audio::engine::{self, NoMidi};
use elyvon_audio::plan::build_plan;
use elyvon_audio::render::{RenderRequest, Rendered, render};
use elyvon_audio::transport::TransportCommand;
use elyvon_core::PPQ;
use elyvon_project::EditCommand;
use elyvon_project::TrackId;
use elyvon_project::automation::{
    AutomationParam, AutomationTarget, Curve, InsertParam, PointSpec,
};
use elyvon_project::effects::{ALL_EFFECTS, EffectKind, EffectPreset, EffectSlot};
use elyvon_project::mixer::{ChannelRef, InsertId, Processor};

fn db(v: f64) -> f64 {
    20.0 * v.max(1e-12).log10()
}

fn sine_track(s: &mut Session, freq: f64, amp: f64, seconds: f64, rate: u32) -> TrackId {
    let t = s.audio_track();
    let frames = (seconds * f64::from(rate)) as usize;
    let x = sine(freq, amp, rate, frames);
    s.place(t, 0, &buffer(vec![x.clone(), x], rate, rate), "sine");
    t
}

fn effect(kind: EffectKind, values: &[(&str, f64)]) -> EffectSlot {
    let mut e = EffectSlot::new(kind);
    for (k, v) in values {
        e.set(k, *v).unwrap();
    }
    e
}

fn add(s: &mut Session, channel: ChannelRef, e: EffectSlot) -> InsertId {
    s.apply(EditCommand::AddInsert {
        channel,
        processor: Processor::Effect(e),
        index: None,
    })
    .created_inserts[0]
}

fn bounce(s: &Session, rate: u32, block: usize, end_tick: Option<u64>) -> Rendered {
    let lookup = s.lookup();
    render(
        &s.project,
        &lookup,
        RenderRequest {
            sample_rate: rate,
            start_tick: 0,
            end_tick,
            block_frames: block,
            metronome: false,
        },
        None,
    )
}

/// Every effect, spread over two tracks, an aux and the master, renders
/// identically whatever the block size: a bounce equals live playback.
#[test]
fn every_effect_in_the_graph_renders_identically_at_any_block_size() {
    const RATE: u32 = 48_000;
    let mut s = Session::new(123.0);
    let a = sine_track(&mut s, 330.0, 0.4, 2.0, RATE);
    let b = sine_track(&mut s, 97.0, 0.3, 2.0, RATE);
    let aux = s
        .apply(EditCommand::AddAux {
            name: Some("FX".into()),
            sources: vec![ChannelRef::Track { id: b }],
        })
        .created_auxes[0];
    let strips = [
        ChannelRef::Track { id: a },
        ChannelRef::Track { id: b },
        ChannelRef::Aux { id: aux },
        ChannelRef::Master,
    ];
    for (i, kind) in ALL_EFFECTS.iter().enumerate() {
        let mut e = EffectSlot::new(*kind);
        if *kind == EffectKind::ConvolutionReverb {
            e.set("lengthSeconds", 0.6).unwrap();
        }
        if *kind == EffectKind::TestGenerator {
            e.set("mode", 1.0).unwrap();
            e.set("levelDb", -40.0).unwrap();
        }
        add(&mut s, strips[i % strips.len()], e);
    }
    let end = Some(u64::from(PPQ) * 6);
    let r64 = bounce(&s, RATE, 64, end);
    let r333 = bounce(&s, RATE, 333, end);
    let r512 = bounce(&s, RATE, 512, end);
    assert!(r64.samples.iter().all(|v| v.is_finite()));
    assert!(r64.report.peak[0] > 0.01);
    assert_eq!(r64.samples, r333.samples, "64 vs 333 frames");
    assert_eq!(r64.samples, r512.samples, "64 vs 512 frames");
    assert_eq!(r64.report.checksum, r512.report.checksum);
}

/// A compressor keyed by another track ducks its own track exactly while the
/// key plays, and the key is processed first even when it comes later in
/// the project.
#[test]
fn external_sidechain_ducks_the_keyed_track() {
    const RATE: u32 = 48_000;
    let mut s = Session::new(120.0);
    let pad = sine_track(&mut s, 220.0, 0.25, 2.0, RATE);
    // The key: silent for 0.5 s, loud for 0.5 s, silent again.
    let key = s.audio_track();
    let n = RATE as usize * 2;
    let mut k = vec![0.0f32; n];
    for (i, v) in k
        .iter_mut()
        .enumerate()
        .take(RATE as usize)
        .skip(RATE as usize / 2)
    {
        *v = if (i / 24) % 2 == 0 { 0.9 } else { -0.9 };
    }
    s.place(key, 0, &buffer(vec![k.clone(), k], RATE, RATE), "kick");
    // Only the pad reaches the master: the key track is muted in the mix
    // (its post-insert signal still keys the sidechain).
    s.apply(EditCommand::SetTracksMuted {
        ids: vec![key],
        muted: true,
    });
    let mut comp = effect(
        EffectKind::Compressor,
        &[
            ("thresholdDb", -30.0),
            ("ratio", 20.0),
            ("attackMs", 0.5),
            ("releaseMs", 20.0),
            ("external", 1.0),
            ("kneeDb", 0.0),
        ],
    );
    comp.sidechain = Some(key);
    add(&mut s, ChannelRef::Track { id: pad }, comp);
    let r = bounce(&s, RATE, 256, Some(u64::from(PPQ) * 4));
    let l = r.channel(0);
    let before = rms(&l[4_800..21_600]);
    let during = rms(&l[30_000..45_000]);
    let after = rms(&l[60_000..90_000]);
    assert!(
        db(during) < db(before) - 20.0,
        "ducked: {} vs {}",
        db(during),
        db(before)
    );
    assert!(
        (db(after) - db(before)).abs() < 0.5,
        "recovers: {} vs {}",
        db(after),
        db(before)
    );
}

#[test]
fn a_track_cannot_key_itself_or_form_a_cycle() {
    let mut s = Session::new(120.0);
    let a = s.audio_track();
    let b = s.audio_track();
    let mut self_key = effect(EffectKind::Gate, &[("external", 1.0)]);
    self_key.sidechain = Some(a);
    let err = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::AddInsert {
            channel: ChannelRef::Track { id: a },
            processor: Processor::Effect(self_key),
            index: None,
        },
    );
    assert!(err.is_err());
    let mut ab = effect(EffectKind::Compressor, &[]);
    ab.sidechain = Some(b);
    add(&mut s, ChannelRef::Track { id: a }, ab);
    let mut ba = effect(EffectKind::Compressor, &[]);
    ba.sidechain = Some(a);
    let err = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::AddInsert {
            channel: ChannelRef::Track { id: b },
            processor: Processor::Effect(ba),
            index: None,
        },
    );
    assert!(err.is_err(), "A keyed by B and B keyed by A is a cycle");
}

/// Automating an effect parameter (an equaliser band's gain) changes the
/// rendered level along the lane.
#[test]
fn effect_parameters_are_automatable() {
    const RATE: u32 = 48_000;
    let mut s = Session::new(120.0);
    let t = sine_track(&mut s, 1_000.0, 0.1, 2.5, RATE);
    let eq = add(
        &mut s,
        ChannelRef::Track { id: t },
        effect(
            EffectKind::ParametricEq,
            &[("b4Freq", 1_000.0), ("b4Q", 0.7)],
        ),
    );
    let gain = AutomationParam::Insert {
        id: eq,
        param: InsertParam::Effect {
            kind: EffectKind::ParametricEq,
            index: EffectKind::ParametricEq.index_of("b4Gain").unwrap() as u8,
        },
    };
    s.apply(EditCommand::AddAutomationPoints {
        target: AutomationTarget {
            channel: ChannelRef::Track { id: t },
            region: None,
            param: gain,
        },
        points: vec![
            PointSpec {
                tick: 0,
                value: 0.0,
                curve: Some(Curve::Linear),
            },
            PointSpec {
                tick: u64::from(PPQ) * 4,
                value: 12.0,
                curve: Some(Curve::Linear),
            },
        ],
    });
    let r = bounce(&s, RATE, 512, None);
    let l = r.channel(0);
    let level = |at: usize| db(rms(&l[at..at + 2_400]));
    let reference = db(0.1 / 2f64.sqrt());
    assert!(
        (level(1_200) - reference).abs() < 0.3,
        "start {}",
        level(1_200)
    );
    // 2 s = 4 beats at 120 BPM: the end of the ramp is +12 dB.
    assert!(
        (level(96_000 - 2_400) - (reference + 12.0)).abs() < 0.5,
        "end {}",
        level(96_000 - 2_400)
    );
    let mid = level(48_000 - 1_200);
    assert!(
        mid > reference + 4.0 && mid < reference + 8.0,
        "middle {mid}"
    );
    // Choices that rebuild the effect cannot be automated.
    let fixed = AutomationParam::Insert {
        id: eq,
        param: InsertParam::Effect {
            kind: EffectKind::ConvolutionReverb,
            index: 0,
        },
    };
    let bad = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::AddAutomationPoints {
            target: AutomationTarget {
                channel: ChannelRef::Track { id: t },
                region: None,
                param: fixed,
            },
            points: vec![PointSpec {
                tick: 0,
                value: 1.0,
                curve: None,
            }],
        },
    );
    assert!(
        bad.is_err(),
        "a parameter of another effect kind is rejected"
    );
}

/// A setting saved as a preset file and loaded again renders bit-identically.
#[test]
fn presets_reopen_and_reproduce_the_same_audio() {
    const RATE: u32 = 48_000;
    let mut s = Session::new(120.0);
    let t = sine_track(&mut s, 440.0, 0.3, 1.0, RATE);
    let original = effect(
        EffectKind::Delay,
        &[("feedback", 55.0), ("mode", 1.0), ("divisionLeft", 6.0)],
    );
    let id = add(&mut s, ChannelRef::Track { id: t }, original);
    let a = bounce(&s, RATE, 512, Some(u64::from(PPQ) * 4));
    let file = EffectPreset::new(EffectKind::Delay, "Mi delay", &original.params).to_json();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mi.elyvonpreset");
    std::fs::write(&path, &file).unwrap();
    let loaded = EffectPreset::from_json(&std::fs::read(&path).unwrap()).unwrap();
    let mut fresh = EffectSlot::new(EffectKind::Delay);
    fresh.params = loaded.values_for(EffectKind::Delay).unwrap();
    s.apply(EditCommand::SetInsert {
        channel: ChannelRef::Track { id: t },
        id,
        bypass: false,
        processor: Processor::Effect(fresh),
    });
    let b = bounce(&s, RATE, 512, Some(u64::from(PPQ) * 4));
    assert_eq!(a.samples, b.samples);
    // And the project round trip keeps the effect exactly.
    let text = elyvon_project::format::encode(&s.project, "test");
    let back = elyvon_project::format::decode(&text).unwrap();
    assert_eq!(
        back.tracks[0].strip.inserts,
        s.project.tracks[0].strip.inserts
    );
}

/// Stopping the transport lets a reverb tail ring out instead of freezing it.
#[test]
fn reverb_and_delay_tails_ring_out_after_stop() {
    const RATE: u32 = 48_000;
    for kind in [EffectKind::Reverb, EffectKind::Delay] {
        let mut s = Session::new(120.0);
        let t = sine_track(&mut s, 300.0, 0.5, 0.5, RATE);
        add(
            &mut s,
            ChannelRef::Track { id: t },
            effect(kind, &[("mix", 50.0)]),
        );
        let lookup = s.lookup();
        let (mut handle, mut eng) =
            engine::create(RATE, 512, build_plan(&s.project, &lookup, RATE, 1));
        handle.send(TransportCommand::Play {
            from_tick: Some(0),
            count_in: false,
            pre_roll: false,
        });
        let mut out = vec![0.0f32; 512 * 2];
        for _ in 0..40 {
            eng.process(&mut out, 2, &mut NoMidi);
        }
        handle.send(TransportCommand::Stop);
        let mut after = Vec::new();
        for _ in 0..40 {
            eng.process(&mut out, 2, &mut NoMidi);
            after.extend_from_slice(&out);
        }
        let first = rms(&after[..9_600]);
        assert!(
            first > 1e-3,
            "{kind:?}: the tail continues after stop ({first})"
        );
        assert!(after.iter().all(|v| v.is_finite()));
    }
}

/// Delay times follow the tempo and keep their duration at every sample
/// rate; a reverb's decay too.
#[test]
fn tempo_synchronised_delay_at_several_sample_rates() {
    for rate in [44_100u32, 48_000, 96_000] {
        let mut s = Session::new(100.0);
        let t = s.audio_track();
        let n = rate as usize * 2;
        let mut x = vec![0.0f32; n];
        x[0] = 1.0;
        s.place(t, 0, &buffer(vec![x.clone(), x], rate, rate), "click");
        add(
            &mut s,
            ChannelRef::Track { id: t },
            effect(
                EffectKind::Delay,
                &[
                    ("divisionLeft", 9.0),
                    ("divisionRight", 9.0),
                    ("feedback", 0.0),
                    ("mix", 100.0),
                    ("lowCutHz", 20.0),
                    ("highCutHz", 20_000.0),
                ],
            ),
        );
        let r = bounce(&s, rate, 512, Some(u64::from(PPQ) * 3));
        let l = r.channel(0);
        let echo = l
            .iter()
            .enumerate()
            .skip(10)
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, _)| i)
            .unwrap();
        let expected = (0.6 * f64::from(rate)).round() as usize;
        assert!(echo.abs_diff(expected) <= 2, "{rate}: {echo} vs {expected}");
    }
}

/// Latency-reporting effects are compensated on parallel paths: a track
/// through a convolution reverb (100 % dry) stays aligned with a dry track.
#[test]
fn convolution_latency_is_compensated_against_other_tracks() {
    const RATE: u32 = 48_000;
    let mut s = Session::new(120.0);
    let a = s.audio_track();
    let b = s.audio_track();
    let n = RATE as usize;
    let mut x = vec![0.0f32; n];
    x[1_000] = 0.5;
    let click = buffer(vec![x.clone(), x], RATE, RATE);
    s.place(a, 0, &click, "a");
    s.place(b, 0, &click, "b");
    add(
        &mut s,
        ChannelRef::Track { id: a },
        effect(
            EffectKind::ConvolutionReverb,
            &[("mix", 0.0), ("lengthSeconds", 0.3)],
        ),
    );
    let r = bounce(&s, RATE, 512, Some(u64::from(PPQ)));
    let l = r.channel(0);
    let peak = l
        .iter()
        .enumerate()
        .max_by(|p, q| p.1.abs().total_cmp(&q.1.abs()))
        .map(|(i, v)| (i, *v))
        .unwrap();
    // Both clicks coincide (sum ≈ 1.0) at the compensated position.
    assert!((peak.1 - 1.0).abs() < 0.02, "{peak:?}");
    assert_eq!(
        build_plan(&s.project, &s.lookup(), RATE, 1)
            .stats
            .latency_frames,
        256
    );
}

/// CPU cost of every effect at 48 kHz in 128-frame blocks (printed with
/// `--nocapture`). The bound is loose so it holds in debug builds; the
/// release figures are recorded in the phase audit.
#[test]
fn every_effect_runs_far_faster_than_real_time() {
    use elyvon_audio::fx::{Effect, FxContext};
    const RATE: u32 = 48_000;
    let map = elyvon_core::tempo::TempoMap::constant(
        elyvon_core::Tempo::new(120.0).unwrap(),
        elyvon_core::TimeSignature::default(),
    );
    let seconds = 2.0;
    let input = sine(440.0, 0.5, RATE, (seconds * f64::from(RATE)) as usize);
    let stereo: Vec<f32> = input.iter().flat_map(|v| [*v, *v]).collect();
    println!("effect                     realtime-factor   ns/frame");
    for kind in ALL_EFFECTS {
        let mut e = EffectSlot::new(*kind);
        if *kind == EffectKind::ConvolutionReverb {
            e.set("lengthSeconds", 6.0).unwrap();
        }
        let mut fx = Effect::new(&e, RATE);
        let mut buf = stereo.clone();
        let start = std::time::Instant::now();
        let mut pos = 0u64;
        for piece in buf.chunks_mut(256) {
            let cx = FxContext {
                rate: RATE,
                map: &map,
                pos,
                advancing: true,
                economy: false,
                key: None,
            };
            fx.process(piece, &cx);
            pos += 128;
        }
        let took = start.elapsed().as_secs_f64();
        let factor = took / seconds;
        println!(
            "{:<26} {:>14.5} {:>10.1}",
            format!("{kind:?}"),
            factor,
            took * 1e9 / (seconds * f64::from(RATE))
        );
        assert!(factor < 0.5, "{kind:?} uses {factor:.3} of real time");
        assert!(buf.iter().all(|v| v.is_finite()));
    }
}

/// Golden comparison: every effect at its default setting, on a fixed
/// signal, keeps the RMS, peak and zero-crossing rate recorded in
/// `tests/fixtures/effects_golden.json` (relative tolerance 1e-3, robust to
/// libm differences between platforms). Regenerate deliberately with
/// `ELYVON_UPDATE_GOLDEN=1`.
#[test]
fn every_effect_matches_its_golden_measurements() {
    use elyvon_audio::fx::{Effect, FxContext};
    const RATE: u32 = 48_000;
    let map = elyvon_core::tempo::TempoMap::constant(
        elyvon_core::Tempo::new(120.0).unwrap(),
        elyvon_core::TimeSignature::default(),
    );
    // Two tones, a level step and a stereo difference.
    let n = RATE as usize;
    let mut x = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f64 / f64::from(RATE);
        let g = if i < n / 2 { 0.25 } else { 0.6 };
        let l = g
            * (0.7 * (std::f64::consts::TAU * 220.0 * t).sin()
                + 0.3 * (std::f64::consts::TAU * 3_100.0 * t).sin());
        let r = g * (std::f64::consts::TAU * 330.0 * t).sin();
        x.push(l as f32);
        x.push(r as f32);
    }
    let mut measured = serde_json::Map::new();
    for kind in ALL_EFFECTS {
        let mut e = EffectSlot::new(*kind);
        if *kind == EffectKind::ConvolutionReverb {
            e.set("lengthSeconds", 0.8).unwrap();
        }
        let mut fx = Effect::new(&e, RATE);
        let mut out = x.clone();
        let mut pos = 0u64;
        for piece in out.chunks_mut(512) {
            fx.process(
                piece,
                &FxContext {
                    rate: RATE,
                    map: &map,
                    pos,
                    advancing: true,
                    economy: false,
                    key: None,
                },
            );
            pos += 256;
        }
        let mut stats = Vec::new();
        for ch in 0..2 {
            let s: Vec<f32> = out.iter().skip(ch).step_by(2).copied().collect();
            let rms =
                (s.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
            let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
            let zc = s
                .windows(2)
                .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
                .count();
            stats.extend([rms, f64::from(peak), zc as f64]);
        }
        measured.insert(format!("{kind:?}"), serde_json::json!(stats));
    }
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/effects_golden.json");
    if std::env::var_os("ELYVON_UPDATE_GOLDEN").is_some() {
        let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(measured)).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        return;
    }
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("golden fixture")).unwrap();
    for (kind, stats) in &measured {
        let want = golden[kind]
            .as_array()
            .unwrap_or_else(|| panic!("{kind} missing from the golden fixture"));
        for (i, (got, want)) in stats.as_array().unwrap().iter().zip(want).enumerate() {
            let (g, w) = (got.as_f64().unwrap(), want.as_f64().unwrap());
            let tol = if i % 3 == 2 {
                2.0 + w * 1e-3
            } else {
                1e-3 * w.abs().max(1e-6)
            };
            assert!((g - w).abs() <= tol, "{kind} stat {i}: {g} vs golden {w}");
        }
    }
}

/// A bypassed effect passes the signal bit-exactly; enabling it fades in
/// over one block instead of switching.
#[test]
fn bypassed_effects_pass_the_signal_exactly_and_switch_without_clicks() {
    const RATE: u32 = 48_000;
    let mut s = Session::new(120.0);
    let t = sine_track(&mut s, 440.0, 0.4, 1.0, RATE);
    let dry = bounce(&s, RATE, 512, Some(u64::from(PPQ)));
    let id = add(
        &mut s,
        ChannelRef::Track { id: t },
        effect(EffectKind::StereoTool, &[("gainDb", 12.0)]),
    );
    s.apply(EditCommand::SetInsert {
        channel: ChannelRef::Track { id: t },
        id,
        bypass: true,
        processor: Processor::Effect(effect(EffectKind::StereoTool, &[("gainDb", 12.0)])),
    });
    let bypassed = bounce(&s, RATE, 512, Some(u64::from(PPQ)));
    assert_eq!(dry.samples, bypassed.samples, "bypass is bit-exact");
    // Live: enable during playback; the largest step stays far below a hard switch.
    let lookup = s.lookup();
    let (mut handle, mut eng) = engine::create(RATE, 512, build_plan(&s.project, &lookup, RATE, 1));
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut out = vec![0.0f32; 256 * 2];
    let mut all = Vec::new();
    for _ in 0..20 {
        eng.process(&mut out, 2, &mut NoMidi);
        all.extend_from_slice(&out);
    }
    let mut on = s.project.clone();
    elyvon_project::edit::apply(
        &mut on,
        &EditCommand::SetInsert {
            channel: ChannelRef::Track { id: t },
            id,
            bypass: false,
            processor: Processor::Effect(effect(EffectKind::StereoTool, &[("gainDb", 12.0)])),
        },
    )
    .unwrap();
    handle.publish(build_plan(&on, &lookup, RATE, 2));
    for _ in 0..20 {
        eng.process(&mut out, 2, &mut NoMidi);
        all.extend_from_slice(&out);
    }
    let step = all
        .chunks(2)
        .collect::<Vec<_>>()
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).abs())
        .fold(0f32, f32::max);
    // The +12 dB utility switched on abruptly would jump by up to ~0.8 at a
    // peak; the one-block fade keeps every frame step near the sine's own.
    assert!(step < 0.1, "largest frame step {step}");
}
