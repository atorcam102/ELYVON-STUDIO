//! Transport, tempo map, metronome, cycle and MIDI scheduling.

// Test helpers outside `#[test]` functions may panic on failure.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use elyvon_audio::contracts::MidiMessage;
use elyvon_audio::engine::{self, NoMidi};
use elyvon_audio::plan::{ClickSounds, build_plan};
use elyvon_audio::render::{RenderRequest, render_stream};
use elyvon_audio::transport::{PlayState, TransportCommand};
use elyvon_core::{MeterChange, Tempo, TempoChange, TimeSignature};
use elyvon_project::{EditCommand, MetronomeSettings, MidiNote, RegionContent, TrackKind};

const RATE: u32 = 48_000;

fn bpm(v: f64) -> Tempo {
    Tempo::new(v).unwrap()
}

/// Independent click-train reference: one click per beat at its frame.
fn reference_clicks(
    beats: &[(u64, bool)],
    clicks: &ClickSounds,
    gain: f32,
    from: u64,
    out: &mut [f32],
) {
    let first = beats.partition_point(|&(at, _)| at + (clicks.len() as u64) <= from);
    for &(at, accent) in &beats[first..] {
        if at >= from + out.len() as u64 {
            break;
        }
        let sound = if accent {
            &clicks.accent
        } else {
            &clicks.normal
        };
        for (k, &v) in sound.iter().enumerate() {
            let t = at + k as u64;
            if t >= from && t < from + out.len() as u64 {
                out[(t - from) as usize] += v * gain;
            }
        }
    }
}

#[test]
fn ten_minute_tempo_map_render_has_exact_length_and_click_train() {
    let mut s = Session::new(120.0);
    let t1 = 4 * 120 * 960;
    let t2 = t1 + 3 * 90 * 960;
    let end = t2 + 3 * 150 * 960;
    s.apply(EditCommand::SetTempoChanges {
        changes: vec![
            TempoChange {
                tick: t1,
                bpm: bpm(90.0),
            },
            TempoChange {
                tick: t2,
                bpm: bpm(150.0),
            },
        ],
    });
    s.apply(EditCommand::SetMetronome {
        settings: MetronomeSettings {
            enabled: true,
            volume_db: 0.0,
            accent: true,
        },
    });
    let map = s.project.transport.tempo_map().unwrap();
    let beats: Vec<(u64, bool)> = map
        .beats(0, end)
        .map(|b| (map.sample_at_tick(b.tick, RATE), b.downbeat))
        .collect();
    assert_eq!(beats.len(), 480 + 270 + 450);
    let clicks = ClickSounds::synthesise(RATE);
    let mut frame = 0u64;
    let mut max_diff = 0.0f32;
    let mut expected = Vec::new();
    let report = render_stream(
        &s.project,
        &|_| None,
        RenderRequest {
            end_tick: Some(end),
            block_frames: 1000,
            metronome: true,
            ..RenderRequest::whole_song(RATE)
        },
        None,
        &mut |block| {
            let n = block.len() / 2;
            expected.clear();
            expected.resize(n, 0.0);
            reference_clicks(&beats, &clicks, 1.0, frame, &mut expected);
            for (i, e) in expected.iter().enumerate() {
                max_diff = max_diff
                    .max((block[i * 2] - e).abs())
                    .max((block[i * 2 + 1] - e).abs());
            }
            frame += n as u64;
        },
    );
    assert_eq!(report.frames, 600 * 48_000, "10 minutes exactly");
    assert_eq!(frame, 600 * 48_000);
    assert_eq!(max_diff, 0.0, "click train differs from the reference");
}

#[test]
fn metronome_follows_odd_and_compound_metres() {
    let mut s = Session::new(100.0);
    s.apply(EditCommand::SetTimeSignature {
        numerator: 3,
        denominator: 4,
    });
    s.apply(EditCommand::SetMeterChanges {
        changes: vec![
            MeterChange {
                bar: 2,
                time_signature: TimeSignature::new(6, 8).unwrap(),
            },
            MeterChange {
                bar: 4,
                time_signature: TimeSignature::new(7, 8).unwrap(),
            },
        ],
    });
    s.apply(EditCommand::SetMetronome {
        settings: MetronomeSettings {
            enabled: true,
            volume_db: -6.0,
            accent: true,
        },
    });
    let map = s.project.transport.tempo_map().unwrap();
    let end = map.bar_start(6);
    let beats: Vec<_> = map.beats(0, end).collect();
    // 2 bars of 3, 2 bars of 6, 2 bars of 7 beats.
    assert_eq!(beats.len(), 2 * 3 + 2 * 6 + 2 * 7);
    assert_eq!(beats.iter().filter(|b| b.downbeat).count(), 6);
    let frames: Vec<(u64, bool)> = beats
        .iter()
        .map(|b| (map.sample_at_tick(b.tick, RATE), b.downbeat))
        .collect();
    let lookup = |_| None;
    let out = elyvon_audio::render::render(
        &s.project,
        &lookup,
        RenderRequest {
            end_tick: Some(end),
            block_frames: 777,
            metronome: true,
            ..RenderRequest::whole_song(RATE)
        },
        None,
    );
    let mut expected = vec![0.0f32; out.frames()];
    let gain = 10f32.powf(-6.0 / 20.0);
    reference_clicks(
        &frames,
        &ClickSounds::synthesise(RATE),
        gain,
        0,
        &mut expected,
    );
    let left = out.channel(0);
    let diff = left
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(diff < 1e-6, "{diff}");
    // Accent clicks are higher pitched and louder.
    let c = ClickSounds::synthesise(RATE);
    assert!(rms(&c.accent) > rms(&c.normal));
}

/// Plays with the real engine (cycle on) and returns the left channel.
fn play_cycle(s: &Session, block: usize, frames: usize) -> Vec<f32> {
    let lookup = s.lookup();
    let plan = build_plan(&s.project, &lookup, RATE, 1);
    let (mut handle, mut eng) = engine::create(RATE, block, plan);
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut out = vec![0.0f32; frames * 2];
    for chunk in out.chunks_mut(block * 2) {
        eng.process(chunk, 2, &mut NoMidi);
    }
    out.iter().step_by(2).copied().collect()
}

#[test]
fn cycle_jumps_are_sample_accurate_at_block_boundaries() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    s.apply(EditCommand::SetTrackPan { id: t, pan: -1.0 });
    // Ramp encodes the source frame: value = frame / 2^20 (exact in f32).
    let ramp: Vec<f32> = (0..200_000).map(|i| i as f32 / (1 << 20) as f32).collect();
    s.place(t, 0, &buffer(vec![ramp], RATE, RATE), "ramp");
    // Cycle 1.1.0 .. 1.3.0 + 7 ticks → frames [0, 48_175) at 25 frames/tick.
    let (a, b) = (0u64, 1920 + 7);
    s.apply(EditCommand::SetLoop {
        enabled: true,
        start: a,
        end: b,
    });
    let (fa, fb) = (a * 25, b * 25);
    let total = 150_000usize;
    let expected: Vec<f32> = (0..total as u64)
        .map(|i| {
            let p = if i < fb { i } else { fa + (i - fb) % (fb - fa) };
            p as f32 / (1 << 20) as f32
        })
        .collect();
    for block in [1usize, 25, 175, 512, (fb - fa) as usize, 4096] {
        let got = play_cycle(&s, block, total);
        let first_bad = got.iter().zip(&expected).position(|(g, e)| g != e);
        assert_eq!(first_bad, None, "block {block}");
    }
}

#[derive(Default)]
struct NoteLog {
    inner: MidiCapture,
}

fn midi_session() -> (Session, elyvon_project::TrackId) {
    let mut s = Session::new(120.0);
    let k = s
        .apply(EditCommand::AddTrack {
            kind: TrackKind::Instrument,
            name: None,
            index: None,
        })
        .created_tracks[0];
    let r = s
        .apply(EditCommand::CreateMidiRegion {
            track_id: k,
            start: 0,
            length: 3840,
        })
        .created_regions[0];
    let (ti, ri) = s.project.find_region(r).unwrap();
    if let RegionContent::Midi { notes } = &mut s.project.tracks[ti].regions[ri].content {
        for (id, (pitch, start, length)) in [
            (60, 0, 480),
            (60, 480, 480), // same pitch retriggered exactly at the previous note-off
            (64, 100, 3000),
            (67, 3500, 1000), // crosses the region end: truncated
            (72, 1919, 2),    // crosses the cycle end below
        ]
        .into_iter()
        .enumerate()
        {
            notes.push(MidiNote {
                id: id as u32 + 1,
                ..MidiNote::new(pitch, 100, start, length)
            });
        }
    }
    (s, k)
}

fn run_midi(
    s: &Session,
    block: usize,
    frames: usize,
    stop_after: bool,
) -> Vec<(u64, usize, MidiMessage)> {
    let plan = build_plan(&s.project, &|_| None, RATE, 1);
    let (mut handle, mut eng) = engine::create(RATE, block, plan);
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut log = NoteLog::default();
    let mut buf = vec![0.0f32; block * 2];
    let mut done = 0;
    while done < frames {
        log.inner.block_start = done as u64;
        eng.process(&mut buf, 2, &mut log.inner);
        done += block;
    }
    if stop_after {
        handle.send(TransportCommand::Stop);
        log.inner.block_start = done as u64;
        eng.process(&mut buf, 2, &mut log.inner);
    }
    log.inner.events
}

fn balance(events: &[(u64, usize, MidiMessage)]) -> [i32; 128] {
    let mut counts = [0i32; 128];
    for (_, _, m) in events {
        match m {
            MidiMessage::NoteOn { pitch, .. } => counts[*pitch as usize] += 1,
            MidiMessage::NoteOff { pitch, .. } => {
                counts[*pitch as usize] -= 1;
                assert!(counts[*pitch as usize] >= 0, "note-off without note-on");
            }
            MidiMessage::ControlChange { .. }
            | MidiMessage::ProgramChange { .. }
            | MidiMessage::PitchBend { .. }
            | MidiMessage::ChannelPressure { .. }
            | MidiMessage::PolyPressure { .. } => {}
        }
    }
    counts
}

#[test]
fn midi_notes_fire_once_on_exact_frames_whatever_the_block_size() {
    let (s, _) = midi_session();
    let reference = run_midi(&s, 512, 120_000, true);
    for block in [1usize, 7, 64, 1000, 4800] {
        let mut ev = run_midi(&s, block, 120_000usize.div_ceil(block) * block, true);
        // Drop events from the over-run tail so lengths match the reference.
        ev.retain(|e| e.0 < 120_000 || matches!(e.2, MidiMessage::NoteOff { .. }));
        let ons: Vec<_> = ev
            .iter()
            .filter(|e| matches!(e.2, MidiMessage::NoteOn { .. }))
            .collect();
        let ref_ons: Vec<_> = reference
            .iter()
            .filter(|e| matches!(e.2, MidiMessage::NoteOn { .. }))
            .collect();
        assert_eq!(ons, ref_ons, "block {block}");
        assert_eq!(balance(&ev), [0; 128], "block {block}: stuck notes");
    }
    let ons: Vec<(u64, u8)> = reference
        .iter()
        .filter_map(|(f, _, m)| match m {
            MidiMessage::NoteOn { pitch, .. } => Some((*f, *pitch)),
            _ => None,
        })
        .collect();
    assert_eq!(
        ons,
        [
            (0, 60),
            (2500, 64),
            (12_000, 60),
            (47_975, 72),
            (87_500, 67)
        ]
    );
    // The note crossing the region end is released at the region end.
    assert!(
        reference
            .iter()
            .any(|(f, _, m)| *f == 96_000 && matches!(m, MidiMessage::NoteOff { pitch: 67, .. }))
    );
}

#[test]
fn cycle_wrap_and_stop_release_sounding_notes() {
    let (mut s, _) = midi_session();
    s.apply(EditCommand::SetLoop {
        enabled: true,
        start: 0,
        end: 1920,
    });
    let ev = run_midi(&s, 333, 200_000, false);
    // Note 72 starts 25 frames before the cycle end and is released at the wrap.
    let wrap = 48_000;
    assert!(
        ev.iter()
            .any(|(f, _, m)| *f == wrap && matches!(m, MidiMessage::NoteOff { pitch: 72, .. }))
    );
    // Long note 64 (beyond the cycle) is released at every wrap and re-struck.
    let on64 = ev
        .iter()
        .filter(|e| matches!(e.2, MidiMessage::NoteOn { pitch: 64, .. }))
        .count();
    assert_eq!(on64, 200_000usize.div_ceil(wrap as usize));
    let ev = run_midi(&s, 333, 30_000, true);
    assert_eq!(balance(&ev), [0; 128], "stop must release everything");
}

#[test]
fn tempo_change_during_playback_keeps_the_musical_position() {
    let s = Session::new(120.0);
    let plan = build_plan(&s.project, &|_| None, RATE, 1);
    let (mut handle, mut eng) = engine::create(RATE, 480, plan);
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut buf = vec![0.0f32; 960];
    for _ in 0..100 {
        eng.process(&mut buf, 2, &mut NoMidi); // 48 000 frames = 1 s = 1920 ticks
    }
    let before = handle.shared().snapshot();
    assert_eq!(before.samples, 48_000);
    assert!((before.ticks - 1920.0).abs() < 1e-6);
    let mut faster = s.project.clone();
    faster.transport.tempo = bpm(150.0);
    handle.publish(build_plan(&faster, &|_| None, RATE, 2));
    eng.process(&mut buf, 2, &mut NoMidi);
    let after = handle.shared().snapshot();
    assert_eq!(after.plan_generation, 2);
    // Same tick (1920) at 150 BPM is 0.8 s → 38 400 frames, plus this block.
    assert_eq!(after.samples, 38_400 + 480);
    assert!((after.ticks - (1920.0 + 480.0 * 150.0 / 60.0 * 960.0 / 48_000.0)).abs() < 1e-6);
    handle.poll();
}

#[test]
fn count_in_pre_roll_and_transport_state_machine() {
    let mut s = Session::new(120.0);
    s.project.transport.count_in_bars = 1;
    s.project.transport.pre_roll_bars = 1;
    let plan = build_plan(&s.project, &|_| None, RATE, 1);
    let (mut handle, mut eng) = engine::create(RATE, 1000, plan);
    let mut buf = vec![0.0f32; 2000];
    let shared = std::sync::Arc::clone(handle.shared());

    // Count-in of one 4/4 bar at 120 BPM = 2 s, then playback from bar 3.
    handle.send(TransportCommand::Play {
        from_tick: Some(7680),
        count_in: true,
        pre_roll: false,
    });
    eng.process(&mut buf, 2, &mut NoMidi);
    assert_eq!(shared.state(), PlayState::CountIn);
    assert!((shared.snapshot().count_in_remaining - (2.0 - 1000.0 / 48_000.0)).abs() < 1e-9);
    assert!(
        buf[0] == 0.0 && buf.iter().any(|v| *v != 0.0),
        "count-in clicks"
    );
    for _ in 0..95 {
        eng.process(&mut buf, 2, &mut NoMidi);
    }
    assert_eq!(shared.state(), PlayState::Playing);
    // 96 blocks = 96 000 frames = exactly the count-in.
    assert_eq!(shared.snapshot().samples, 4 * 48_000);
    eng.process(&mut buf, 2, &mut NoMidi);
    assert_eq!(shared.snapshot().samples, 4 * 48_000 + 1000);

    // Pause keeps the position, Stop then Stop returns to the play start.
    handle.send(TransportCommand::Pause);
    eng.process(&mut buf, 2, &mut NoMidi);
    assert_eq!(shared.state(), PlayState::Paused);
    let paused = shared.snapshot().samples;
    eng.process(&mut buf, 2, &mut NoMidi);
    assert_eq!(shared.snapshot().samples, paused);
    handle.send(TransportCommand::Stop);
    handle.send(TransportCommand::Stop);
    eng.process(&mut buf, 2, &mut NoMidi);
    assert_eq!(shared.state(), PlayState::Stopped);
    assert_eq!(shared.snapshot().samples, 4 * 48_000);

    // Pre-roll: start one bar before bar 3, same position in the bar.
    handle.send(TransportCommand::Play {
        from_tick: Some(7680 + 960),
        count_in: false,
        pre_roll: true,
    });
    eng.process(&mut buf, 2, &mut NoMidi);
    assert_eq!(shared.snapshot().samples, (3840 + 960) * 25 + 1000);

    // Locate while playing.
    handle.send(TransportCommand::Locate { tick: 960 });
    eng.process(&mut buf, 2, &mut NoMidi);
    assert_eq!(shared.snapshot().samples, 960 * 25 + 1000);
    assert!((shared.snapshot().ticks - (960.0 + 40.0)).abs() < 1e-9);
}
