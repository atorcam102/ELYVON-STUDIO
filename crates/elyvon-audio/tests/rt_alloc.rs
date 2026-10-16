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
                pitch: 40 + (i % 40) as u8,
                velocity: 90,
                start: i * 60,
                length: 200,
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
