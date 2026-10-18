//! Real-time safety: the audio callback must not allocate or free memory,
//! whatever the transport does (play, locate, pause, count-in, cycle wraps,
//! plan swaps, MIDI). Verified with a counting global allocator that is armed
//! only on the thread and during the calls under test.
// A GlobalAlloc implementation is inherently `unsafe`; it only forwards to the
// system allocator and is confined to this test binary.
#![allow(unsafe_code)]
// Test helpers outside `#[test]` functions may panic on failure.
#![allow(clippy::unwrap_used)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use elyvon_audio::engine;
use elyvon_audio::plan::build_plan;
use elyvon_audio::transport::TransportCommand;
use elyvon_project::{EditCommand, MetronomeSettings, MidiNote, RegionContent, TrackKind};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn armed() -> bool {
    ARMED.try_with(Cell::get).unwrap_or(false)
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if armed() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if armed() {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if armed() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn guarded(f: impl FnOnce()) {
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
}

#[test]
fn process_never_allocates_or_frees() {
    const RATE: u32 = 48_000;
    // The detector itself works: an armed allocation is counted.
    guarded(|| drop(std::hint::black_box(vec![1u8; 16])));
    assert_eq!(
        (
            ALLOCS.swap(0, Ordering::Relaxed),
            FREES.swap(0, Ordering::Relaxed)
        ),
        (1, 1)
    );

    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let tone = buffer(
        vec![
            sine(440.0, 0.5, RATE, 96_000),
            sine(550.0, 0.5, RATE, 96_000),
        ],
        RATE,
        RATE,
    );
    let a = s.place(t, 0, &tone, "a");
    s.place(t, 960, &tone, "b"); // overlaps a
    s.apply(EditCommand::SetAudioRegion {
        id: a,
        gain_db: -3.0,
        fade_in: 500,
        fade_out: 500,
        looped: true,
    });
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
        for i in 0..64u64 {
            notes.push(MidiNote {
                id: i as u32 + 1,
                ..MidiNote::new(40 + (i % 40) as u8, 90, i * 60, 200)
            });
        }
    }
    s.apply(EditCommand::SetMetronome {
        settings: MetronomeSettings {
            enabled: true,
            volume_db: -6.0,
            accent: true,
        },
    });
    s.apply(EditCommand::SetLoop {
        enabled: true,
        start: 0,
        end: 3840,
    });
    s.project.transport.count_in_bars = 1;

    let lookup = s.lookup();
    let plan = build_plan(&s.project, &lookup, RATE, 1);
    let (mut handle, mut eng) = engine::create(RATE, 256, plan);
    let mut out = vec![0.0f32; 512 * 2];
    let mut midi = elyvon_audio::engine::NoMidi;

    let step = |eng: &mut engine::Engine,
                out: &mut [f32],
                midi: &mut elyvon_audio::engine::NoMidi,
                n: usize| {
        for _ in 0..n {
            guarded(|| eng.process(out, 2, midi));
        }
    };

    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    step(&mut eng, &mut out, &mut midi, 500); // several cycle wraps
    handle.send(TransportCommand::Locate { tick: 1234 });
    step(&mut eng, &mut out, &mut midi, 10);
    // A tempo change publishes a new plan (allocated here, on the control side).
    let mut edited = s.project.clone();
    edited.transport.tempo = elyvon_core::Tempo::new(97.0).unwrap();
    handle.publish(build_plan(&edited, &lookup, RATE, 2));
    step(&mut eng, &mut out, &mut midi, 10);
    handle.poll(); // frees the replaced plan on this (control) side, unarmed
    handle.send(TransportCommand::Pause);
    step(&mut eng, &mut out, &mut midi, 5);
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: true,
        pre_roll: false,
    });
    step(&mut eng, &mut out, &mut midi, 400);
    handle.send(TransportCommand::Stop);
    handle.send(TransportCommand::Stop);
    step(&mut eng, &mut out, &mut midi, 5);
    // Odd callback sizes larger than the internal block.
    let mut big = vec![0.0f32; 1031 * 2];
    handle.send(TransportCommand::Play {
        from_tick: Some(100),
        count_in: false,
        pre_roll: false,
    });
    for _ in 0..50 {
        guarded(|| eng.process(&mut big, 2, &mut midi));
    }

    // Input capture and direct monitoring use the same guarded allocator.
    let dir = tempfile::tempdir().unwrap();
    let config = elyvon_audio::recording::CaptureConfig {
        rate: RATE,
        input_channels: 1,
        start_frame: 0,
        punch: None,
        cycle: None,
        compensation_frames: 0,
        monitor_only: false,
        max_bytes: None,
    };
    let routes = vec![elyvon_audio::recording::CaptureRoute {
        track: elyvon_project::TrackId::new(),
        input: 0,
        channels: 1,
        armed: true,
        monitor: true,
        gain: 1.0,
    }];
    let (mut tap, writer) = elyvon_audio::recording::RecordingWriter::start(
        dir.path(),
        config,
        routes,
        handle.take_monitor_input(),
    )
    .unwrap();
    let input = vec![0.125f32; 256];
    for n in 0..12 {
        guarded(|| {
            tap.process(
                &input,
                n * 256,
                elyvon_audio::transport::PlayState::Playing,
                0,
            )
        });
        guarded(|| eng.process(&mut out[..512], 2, &mut midi));
    }
    drop(tap);
    let report = writer.finish(false);
    assert_eq!(
        report.state,
        elyvon_audio::recording::RecordingState::Finalized
    );
    // Disk-backed playback reads only atomics inside the callback.
    let streamed = std::sync::Arc::new(
        elyvon_audio::media::MediaBuffer::load_streamed(
            &common::fixture("tone.flac"),
            RATE,
            2 << 20,
        )
        .unwrap(),
    );
    let mut streamed_session = Session::new(120.0);
    let track = streamed_session.audio_track();
    streamed_session.place(track, 0, &streamed, "stream");
    let plan = build_plan(
        &streamed_session.project,
        &streamed_session.lookup(),
        RATE,
        1,
    );
    let (mut stream_handle, mut stream_engine) = engine::create(RATE, 256, plan);
    stream_handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    for _ in 0..20 {
        stream_engine
            .prepare_media(stream_engine.position(), 256)
            .unwrap();
        guarded(|| stream_engine.process(&mut out[..512], 2, &mut midi));
    }
    let mut delay = elyvon_audio::latency::DelayLine::new(128).unwrap();
    guarded(|| {
        for _ in 0..512 {
            std::hint::black_box(delay.process(0.25));
        }
    });
    assert!(
        out.iter().chain(&big).any(|v| *v != 0.0),
        "the engine produced audio"
    );
    assert_eq!(
        ALLOCS.load(Ordering::Relaxed),
        0,
        "allocations in the audio callback"
    );
    assert_eq!(
        FREES.load(Ordering::Relaxed),
        0,
        "frees in the audio callback"
    );
    handle.poll();
}

#[test]
fn mixer_graph_instruments_and_swaps_never_allocate() {
    use elyvon_audio::engine::LiveMidi;
    use elyvon_project::mixer::{ChannelRef, FilterMode, Processor, Send};
    use elyvon_project::mixer_edit::StripPatch;
    const RATE: u32 = 48_000;
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let tone = buffer(vec![sine(440.0, 0.5, RATE, 96_000)], RATE, RATE);
    s.place(t, 0, &tone, "tone");
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
            length: 7680,
        })
        .created_regions[0];
    s.apply(EditCommand::AddNotes {
        region_id: r,
        notes: (0..48)
            .map(|i| elyvon_project::midi_edit::NoteSpec {
                pitch: 48 + (i % 24) as u8,
                velocity: 100,
                channel: (i % 3) as u8,
                start: i * 160,
                length: 900,
            })
            .collect(),
    });
    let track = ChannelRef::Track { id: t };
    let keys = ChannelRef::Track { id: k };
    let aux = s
        .apply(EditCommand::AddAux {
            name: Some("Verb".into()),
            sources: vec![track],
        })
        .created_auxes[0];
    let bus = s
        .apply(EditCommand::AddAux {
            name: Some("Bus".into()),
            sources: vec![],
        })
        .created_auxes[0];
    s.apply(EditCommand::SetStrip {
        channel: keys,
        patch: StripPatch {
            output: Some(elyvon_project::mixer::OutputTarget::Aux { id: bus }),
            ..StripPatch::default()
        },
    });
    s.apply(EditCommand::SetSend {
        channel: keys,
        target: aux,
        send: Some(Send {
            target: aux,
            level_db: -6.0,
            pre_fader: true,
            muted: false,
        }),
    });
    for (channel, p) in [
        (
            track,
            Processor::Limiter {
                ceiling_db: -3.0,
                lookahead_ms: 5.0,
                release_ms: 50.0,
            },
        ),
        (track, Processor::Trim { gain_db: -2.0 }),
        (
            ChannelRef::Aux { id: aux },
            Processor::Filter {
                mode: FilterMode::LowPass,
                cutoff_hz: 3000.0,
                q: 0.7,
            },
        ),
        (
            ChannelRef::Aux { id: bus },
            Processor::Utility {
                invert_left: true,
                invert_right: false,
                swap: true,
                mono: false,
            },
        ),
        (
            ChannelRef::Master,
            Processor::Limiter {
                ceiling_db: -1.0,
                lookahead_ms: 2.0,
                release_ms: 100.0,
            },
        ),
    ] {
        s.apply(EditCommand::AddInsert {
            channel,
            processor: p,
            index: None,
        });
    }
    s.apply(EditCommand::SetLoop {
        enabled: true,
        start: 0,
        end: 7680,
    });
    let lookup = s.lookup();
    let (mut handle, mut eng) = engine::create(RATE, 512, build_plan(&s.project, &lookup, RATE, 1));
    let mut out = vec![0.0f32; 512 * 2];
    let mut midi = elyvon_audio::engine::NoMidi;
    let key = k.0.as_u128();
    // Stopped: live notes still sound through the graph.
    assert!(handle.send_live(LiveMidi::new(key, &[0x90, 60, 100]).unwrap()));
    for _ in 0..20 {
        guarded(|| eng.process(&mut out, 2, &mut midi));
    }
    assert!(
        out.iter().any(|v| *v != 0.0),
        "live note heard while stopped"
    );
    handle.send_live(LiveMidi::new(key, &[0x80, 60, 0]).unwrap());
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    for round in 0..6u32 {
        for _ in 0..60 {
            guarded(|| eng.process(&mut out, 2, &mut midi));
        }
        // Graph swaps while playing: fader, solo, insert order and bypass.
        let mut edited = s.project.clone();
        let inserts = edited.tracks[0].strip.inserts.clone();
        let mut apply = |c| elyvon_project::edit::apply(&mut edited, &c).unwrap();
        apply(EditCommand::SetTrackVolume {
            id: t,
            volume_db: -f64::from(round),
        });
        apply(EditCommand::SetTracksSoloed {
            ids: vec![k],
            soloed: round % 2 == 1,
        });
        apply(EditCommand::MoveInsert {
            channel: track,
            id: inserts[0].id,
            to_index: 1,
        });
        apply(EditCommand::SetInsert {
            channel: track,
            id: inserts[1].id,
            bypass: round % 2 == 0,
            processor: inserts[1].processor,
        });
        handle.publish(build_plan(&edited, &lookup, RATE, 2 + u64::from(round)));
        handle.send_live(
            LiveMidi::new(key, &[0xb0, 64, if round % 2 == 0 { 127 } else { 0 }]).unwrap(),
        );
        guarded(|| eng.process(&mut out, 2, &mut midi));
        handle.poll();
    }
    handle.send(TransportCommand::Locate { tick: 960 });
    let mut big = vec![0.0f32; 3000 * 2];
    for _ in 0..30 {
        guarded(|| eng.process(&mut big, 2, &mut midi));
    }
    assert!(big.iter().all(|v| v.is_finite()));
    assert_eq!(
        ALLOCS.load(Ordering::Relaxed),
        0,
        "allocations in the audio callback"
    );
    assert_eq!(
        FREES.load(Ordering::Relaxed),
        0,
        "frees in the audio callback"
    );
    handle.poll();
}

#[test]
fn automation_envelopes_and_live_overrides_never_allocate() {
    use elyvon_audio::automation::{ParamOverride, StripKey};
    use elyvon_project::automation::{
        AutomationMode, AutomationParam, AutomationTarget, Curve, InsertParam, PointSpec,
    };
    use elyvon_project::automation_record::TouchPhase;
    use elyvon_project::mixer::{ChannelRef, FilterMode, Processor, Send};
    const RATE: u32 = 48_000;
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    let tone = buffer(vec![sine(440.0, 0.5, RATE, 96_000)], RATE, RATE);
    s.place(t, 0, &tone, "tone");
    let track = ChannelRef::Track { id: t };
    let aux = s
        .apply(EditCommand::AddAux {
            name: Some("Verb".into()),
            sources: vec![],
        })
        .created_auxes[0];
    s.apply(EditCommand::SetSend {
        channel: track,
        target: aux,
        send: Some(Send {
            target: aux,
            level_db: -6.0,
            pre_fader: false,
            muted: false,
        }),
    });
    let mut ids = Vec::new();
    for p in [
        Processor::Trim { gain_db: 0.0 },
        Processor::Filter {
            mode: FilterMode::LowPass,
            cutoff_hz: 3000.0,
            q: 0.7,
        },
        Processor::Limiter {
            ceiling_db: -3.0,
            lookahead_ms: 5.0,
            release_ms: 50.0,
        },
    ] {
        ids.push(
            s.apply(EditCommand::AddInsert {
                channel: track,
                processor: p,
                index: None,
            })
            .created_inserts[0],
        );
    }
    let lanes: Vec<(ChannelRef, AutomationParam)> = vec![
        (track, AutomationParam::Volume),
        (track, AutomationParam::Pan),
        (track, AutomationParam::Mute),
        (track, AutomationParam::SendLevel { aux }),
        (
            track,
            AutomationParam::Insert {
                id: ids[0],
                param: InsertParam::TrimGain,
            },
        ),
        (
            track,
            AutomationParam::Insert {
                id: ids[1],
                param: InsertParam::FilterCutoff,
            },
        ),
        (
            track,
            AutomationParam::Insert {
                id: ids[2],
                param: InsertParam::Bypass,
            },
        ),
        (ChannelRef::Aux { id: aux }, AutomationParam::Volume),
        (ChannelRef::Master, AutomationParam::Volume),
    ];
    for (channel, param) in &lanes {
        let spec = param.spec();
        s.apply(EditCommand::AddAutomationPoints {
            target: AutomationTarget {
                channel: *channel,
                region: None,
                param: *param,
            },
            points: (0..40)
                .map(|k| PointSpec {
                    tick: k * 200,
                    value: if k % 2 == 0 {
                        spec.min
                    } else {
                        spec.max.min(spec.min + 30.0)
                    },
                    curve: Some(if k % 3 == 0 {
                        Curve::Step
                    } else {
                        Curve::Bezier { c1: 0.2, c2: 0.9 }
                    }),
                })
                .collect(),
        });
    }
    s.apply(EditCommand::SetAutomationMode {
        channel: track,
        mode: AutomationMode::Touch,
    });
    s.apply(EditCommand::SetLoop {
        enabled: true,
        start: 0,
        end: 7680,
    });
    let lookup = s.lookup();
    let (mut handle, mut eng) = engine::create(RATE, 512, build_plan(&s.project, &lookup, RATE, 1));
    let mut out = vec![0.0f32; 512 * 2];
    let mut midi = elyvon_audio::engine::NoMidi;
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let key = StripKey::Track(t.0.as_u128());
    for round in 0..8u32 {
        let phase = match round % 3 {
            0 => TouchPhase::Begin,
            1 => TouchPhase::Move,
            _ => TouchPhase::End,
        };
        for param in [AutomationParam::Volume, AutomationParam::SendLevel { aux }] {
            handle.send_override(ParamOverride {
                strip: key,
                param,
                value: -f64::from(round),
                phase,
            });
        }
        for _ in 0..40 {
            guarded(|| eng.process(&mut out, 2, &mut midi));
        }
        // Lane edits while playing swap the plan.
        let mut edited = s.project.clone();
        elyvon_project::edit::apply(
            &mut edited,
            &EditCommand::AddAutomationPoints {
                target: AutomationTarget {
                    channel: track,
                    region: None,
                    param: AutomationParam::Volume,
                },
                points: vec![PointSpec {
                    tick: 100 + u64::from(round),
                    value: -3.0,
                    curve: None,
                }],
            },
        )
        .unwrap();
        handle.publish(build_plan(&edited, &lookup, RATE, 2 + u64::from(round)));
        guarded(|| eng.process(&mut out, 2, &mut midi));
        handle.poll();
    }
    handle.send(TransportCommand::Stop);
    for _ in 0..4 {
        guarded(|| eng.process(&mut out, 2, &mut midi));
    }
    assert!(out.iter().all(|v| v.is_finite()));
    assert_eq!(
        ALLOCS.load(Ordering::Relaxed),
        0,
        "allocations in the audio callback"
    );
    assert_eq!(
        FREES.load(Ordering::Relaxed),
        0,
        "frees in the audio callback"
    );
    handle.poll();
}

#[test]
fn native_effects_sidechain_and_analyser_never_allocate() {
    use elyvon_project::automation::{AutomationParam, AutomationTarget, InsertParam, PointSpec};
    use elyvon_project::effects::{ALL_EFFECTS, EffectKind, EffectSlot};
    use elyvon_project::mixer::{ChannelRef, Processor};
    const RATE: u32 = 48_000;
    let mut s = Session::new(118.0);
    let a = s.audio_track();
    let b = s.audio_track();
    let tone = buffer(vec![sine(220.0, 0.5, RATE, 96_000)], RATE, RATE);
    s.place(a, 0, &tone, "a");
    s.place(b, 0, &tone, "b");
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
    let mut ids = Vec::new();
    for (i, kind) in ALL_EFFECTS.iter().enumerate() {
        let mut e = EffectSlot::new(*kind);
        if *kind == EffectKind::ConvolutionReverb {
            e.set("lengthSeconds", 1.0).unwrap();
        }
        if *kind == EffectKind::Compressor {
            // The compressor lands on track B; track A keys it.
            e.set("external", 1.0).unwrap();
            e.sidechain = Some(a);
        }
        let channel = strips[i % strips.len()];
        let id = s
            .apply(EditCommand::AddInsert {
                channel,
                processor: Processor::Effect(e),
                index: None,
            })
            .created_inserts[0];
        ids.push((channel, id, e));
    }
    // Automate the equaliser's band-4 gain (an effect parameter).
    let (eq_channel, eq_id, _) = ids[0];
    s.apply(EditCommand::AddAutomationPoints {
        target: AutomationTarget {
            channel: eq_channel,
            region: None,
            param: AutomationParam::Insert {
                id: eq_id,
                param: InsertParam::Effect {
                    kind: EffectKind::ParametricEq,
                    index: 21,
                },
            },
        },
        points: vec![
            PointSpec {
                tick: 0,
                value: -12.0,
                curve: None,
            },
            PointSpec {
                tick: 3840,
                value: 12.0,
                curve: None,
            },
        ],
    });
    let lookup = s.lookup();
    let (mut handle, mut eng) = engine::create(RATE, 512, build_plan(&s.project, &lookup, RATE, 1));
    handle.shared().set_analyzer_slot(Some(0));
    let mut out = vec![0.0f32; 480 * 2];
    let mut midi = elyvon_audio::engine::NoMidi;
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    for round in 0..6u32 {
        for _ in 0..30 {
            guarded(|| eng.process(&mut out, 2, &mut midi));
        }
        // Graph swaps while playing: effect parameters, bypass, order.
        let mut edited = s.project.clone();
        for (k, (channel, id, e)) in ids.iter().enumerate() {
            let mut e = *e;
            for (i, d) in e.kind.params().iter().enumerate() {
                if d.automatable && !d.discrete() && (i + k + round as usize).is_multiple_of(3) {
                    e.params.0[i] = d.min + (d.max - d.min) * 0.5;
                }
            }
            elyvon_project::edit::apply(
                &mut edited,
                &EditCommand::SetInsert {
                    channel: *channel,
                    id: *id,
                    bypass: (k + round as usize).is_multiple_of(5),
                    processor: Processor::Effect(e),
                },
            )
            .unwrap();
        }
        handle.publish(build_plan(&edited, &lookup, RATE, 2 + u64::from(round)));
        guarded(|| eng.process(&mut out, 2, &mut midi));
        handle.poll();
    }
    // Stopped: tails keep ringing through the graph.
    handle.send(TransportCommand::Stop);
    for _ in 0..20 {
        guarded(|| eng.process(&mut out, 2, &mut midi));
    }
    let mut big = vec![0.0f32; 4096 * 2];
    guarded(|| eng.process(&mut big, 2, &mut midi));
    assert!(out.iter().chain(&big).all(|v| v.is_finite()));
    assert_eq!(
        ALLOCS.load(Ordering::Relaxed),
        0,
        "allocations in the audio callback"
    );
    assert_eq!(
        FREES.load(Ordering::Relaxed),
        0,
        "frees in the audio callback"
    );
    let meters = handle.shared().read_meters(&mut Default::default(), 2, 1);
    assert!(meters.reduction.iter().all(|r| r.reduction_db.is_finite()));
    handle.poll();
}
