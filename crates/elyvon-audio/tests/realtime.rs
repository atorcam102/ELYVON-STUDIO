//! Headless verification of the real-time engine core. No device, no clock.
//!
//! * Buffer sizes 64/128/256/512 × sample rates 44.1/48/96 kHz.
//! * No heap allocation or deallocation inside the callback path, checked by
//!   a counting global allocator for every supported device sample type.
//! * Determinism, buffer-size independence, transport/clock, tone accuracy,
//!   meters, clipping and the lock-free command ring buffer.

// The counting allocator needs `unsafe impl GlobalAlloc`; it only delegates to
// the system allocator. Test-only: production code keeps `unsafe_code = deny`.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use elyvon_audio::command::{Command, TestTone, ToneChannels};
use elyvon_audio::format::OutputSample;
use elyvon_audio::gain::db_to_gain;
use elyvon_audio::offline::{OfflineRenderer, analysis};
use elyvon_audio::render::MAX_BLOCK_FRAMES;

struct CountingAllocator;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static EVENTS: Cell<u64> = const { Cell::new(0) };
}

fn record() {
    // `try_with` so allocations during thread teardown never panic.
    let _ = TRACKING.try_with(|t| {
        if t.get() {
            let _ = EVENTS.try_with(|e| e.set(e.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `f` and returns how many heap operations it performed on this thread.
fn heap_events(f: impl FnOnce()) -> u64 {
    EVENTS.with(|e| e.set(0));
    TRACKING.with(|t| t.set(true));
    f();
    TRACKING.with(|t| t.set(false));
    EVENTS.with(|e| e.get())
}

const BUFFERS: [usize; 4] = [64, 128, 256, 512];
const RATES: [u32; 3] = [44_100, 48_000, 96_000];

fn tone(level_db: f32, frequency_hz: f32) -> Command {
    Command::SetTestTone(TestTone {
        enabled: true,
        frequency_hz,
        level_db,
        channels: ToneChannels::All,
    })
}

#[test]
fn allocator_detector_is_live() {
    // Guard against a silently broken detector making every test pass.
    let n = heap_events(|| {
        let v = vec![1u8; 32];
        std::hint::black_box(&v);
    });
    assert!(n >= 2, "counting allocator did not observe a Vec, saw {n}");
}

fn assert_callback_never_allocates<S: OutputSample + std::fmt::Debug>(rate: u32, buffer: usize) {
    let mut engine = OfflineRenderer::new(rate, 2);
    let mut device = vec![S::SILENCE; buffer * 2];
    // Commands are queued (control side) before tracking starts; the callback
    // pops and applies them inside the tracked region.
    engine.send(tone(-12.0, 997.0)).expect("queue has room");
    engine.send(Command::Play).expect("queue has room");
    let events = heap_events(|| {
        for _ in 0..200 {
            engine.core_mut().process_interleaved(&mut device, 2);
        }
    });
    assert_eq!(
        events,
        0,
        "{} callback at {rate} Hz / {buffer} frames allocated",
        std::any::type_name::<S>()
    );
    assert!(engine.core().is_playing());
}

#[test]
fn callback_never_allocates_for_any_buffer_rate_or_sample_format() {
    for rate in RATES {
        for buffer in BUFFERS {
            assert_callback_never_allocates::<f32>(rate, buffer);
            assert_callback_never_allocates::<f64>(rate, buffer);
            assert_callback_never_allocates::<i16>(rate, buffer);
            assert_callback_never_allocates::<i32>(rate, buffer);
            assert_callback_never_allocates::<u16>(rate, buffer);
            assert_callback_never_allocates::<u8>(rate, buffer);
        }
    }
}

#[test]
fn planar_path_and_oversized_device_blocks_never_allocate() {
    let mut engine = OfflineRenderer::new(48_000, 2);
    let mut left = vec![0.0f32; MAX_BLOCK_FRAMES * 3 + 17];
    let mut right = left.clone();
    let mut interleaved = vec![0.0f32; (MAX_BLOCK_FRAMES * 4 + 5) * 2];
    engine.send(tone(-18.0, 440.0)).unwrap();
    let events = heap_events(|| {
        let mut outs: [&mut [f32]; 2] = [&mut left, &mut right];
        engine.core_mut().process_planar(&mut outs);
        engine.core_mut().process_interleaved(&mut interleaved, 2);
    });
    assert_eq!(events, 0);
    assert!(analysis::peak(&left) > 0.0);
}

#[test]
fn command_burst_is_bounded_and_allocation_free() {
    let mut engine = OfflineRenderer::new(48_000, 1);
    for frame in 0..200u64 {
        engine.send(Command::Locate { frame }).unwrap();
    }
    let mut device = vec![0.0f32; 64];
    let events = heap_events(|| engine.core_mut().process_interleaved(&mut device, 1));
    assert_eq!(events, 0);
    // At most MAX_COMMANDS_PER_BLOCK (64) commands per chunk: not all applied yet.
    assert_eq!(engine.core().transport(), 63);
    for _ in 0..4 {
        engine.core_mut().process_interleaved(&mut device, 1);
    }
    assert_eq!(engine.core().transport(), 199);
}

#[test]
fn stopped_engine_renders_exact_silence_and_advances_only_the_clock() {
    for rate in RATES {
        for buffer in BUFFERS {
            let mut engine = OfflineRenderer::new(rate, 2);
            let out = engine.render(rate as usize / 10, buffer);
            assert!(out.iter().flatten().all(|v| *v == 0.0));
            assert_eq!(engine.core().clock(), u64::from(rate) / 10);
            assert_eq!(engine.core().transport(), 0);
            let snap = engine.snapshot();
            assert_eq!(snap.output_peak_db, vec![-96.0, -96.0]);
            assert_eq!(snap.stats.clipped_samples, 0);
            assert!(!snap.playing && !snap.tone_active);
        }
    }
}

#[test]
fn transport_play_stop_locate_is_sample_accurate() {
    for rate in RATES {
        for buffer in BUFFERS {
            let mut engine = OfflineRenderer::new(rate, 2);
            engine.render(buffer * 3, buffer);
            engine.send(Command::Play).unwrap();
            engine.render(buffer * 10, buffer);
            assert_eq!(engine.core().transport(), (buffer * 10) as u64);
            engine.send(Command::Stop).unwrap();
            engine.render(buffer * 2, buffer);
            assert_eq!(
                engine.core().transport(),
                (buffer * 10) as u64,
                "stop keeps position"
            );
            engine.send(Command::Locate { frame: 1000 }).unwrap();
            engine.send(Command::Play).unwrap();
            engine.render(buffer, buffer);
            assert_eq!(engine.core().transport(), 1000 + buffer as u64);
            // The monotonic clock counts every rendered frame, playing or not.
            assert_eq!(engine.core().clock(), (buffer * 16) as u64);
            let snap = engine.snapshot();
            assert_eq!(snap.clock_frames, engine.core().clock());
            assert_eq!(snap.transport_frames, engine.core().transport());
            assert!(snap.playing);
        }
    }
}

#[test]
fn test_tone_has_the_requested_frequency_level_and_meter_reading() {
    for rate in RATES {
        for buffer in BUFFERS {
            let mut engine = OfflineRenderer::new(rate, 2);
            engine.send(tone(-12.0, 1000.0)).unwrap();
            // Skip the 10 ms ramp, then analyse half a second.
            engine.render(rate as usize / 50, buffer);
            let _ = engine.snapshot();
            let out = engine.render(rate as usize / 2, buffer);
            let expected_peak = db_to_gain(-12.0) as f32;
            for channel in &out {
                let peak = analysis::peak(channel);
                assert!(
                    (peak - expected_peak).abs() < 1e-3,
                    "peak {peak} at {rate}/{buffer}"
                );
                let rms = analysis::rms(channel);
                assert!((rms - f64::from(expected_peak) / 2f64.sqrt()).abs() < 1e-3);
                let f = analysis::estimate_frequency(channel, rate).unwrap();
                assert!(
                    (f - 1000.0).abs() < 0.05,
                    "frequency {f} at {rate}/{buffer}"
                );
            }
            let snap = engine.snapshot();
            assert!(snap.tone_active);
            for db in snap.output_peak_db {
                assert!((db + 12.0).abs() < 0.01, "meter {db} dBFS");
            }
            assert_eq!(snap.stats.clipped_samples, 0);
        }
    }
}

#[test]
fn output_is_bit_identical_for_every_buffer_size() {
    for rate in RATES {
        let reference = {
            let mut engine = OfflineRenderer::new(rate, 2);
            engine.send(tone(-18.0, 440.0)).unwrap();
            engine.send(Command::Play).unwrap();
            engine.render(8192 + 77, 8192 + 77)
        };
        for buffer in BUFFERS {
            let mut engine = OfflineRenderer::new(rate, 2);
            engine.send(tone(-18.0, 440.0)).unwrap();
            engine.send(Command::Play).unwrap();
            assert_eq!(
                engine.render(8192 + 77, buffer),
                reference,
                "{rate}/{buffer}"
            );
        }
    }
}

#[test]
fn tone_ramps_in_and_out_without_clicks() {
    let rate = 48_000;
    let mut engine = OfflineRenderer::new(rate, 1);
    engine.send(tone(-6.0, 100.0)).unwrap();
    let on = engine.render(960, 64).remove(0);
    // A 10 ms ramp: the first samples are tiny, sample-to-sample steps bounded.
    assert!(on[0].abs() < 1e-3);
    let max_step = on
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step < 0.02, "max step {max_step}");
    engine
        .send(Command::SetTestTone(TestTone {
            enabled: false,
            ..TestTone::default()
        }))
        .unwrap();
    let off = engine.render(960, 64).remove(0);
    let max_step = off
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step < 0.02);
    assert!(
        off[480..].iter().all(|v| *v == 0.0),
        "silent after the ramp"
    );
    assert!(!engine.snapshot().tone_active);
}

#[test]
fn tone_can_target_a_single_channel_for_left_right_checks() {
    let mut engine = OfflineRenderer::new(48_000, 2);
    engine
        .send(Command::SetTestTone(TestTone {
            enabled: true,
            frequency_hz: 440.0,
            level_db: -18.0,
            channels: ToneChannels::Only { channel: 1 },
        }))
        .unwrap();
    let out = engine.render(4800, 128);
    assert_eq!(analysis::peak(&out[0]), 0.0);
    assert!(analysis::peak(&out[1]) > 0.1);
    let snap = engine.snapshot();
    assert_eq!(snap.output_peak_db[0], -96.0);
    assert!(snap.output_peak_db[1] > -19.0);
}

#[test]
fn mono_and_multichannel_devices_get_defined_samples() {
    // Mono device.
    let mut mono = OfflineRenderer::new(44_100, 1);
    mono.send(tone(-6.0, 440.0)).unwrap();
    let mut buf = vec![0i16; 512];
    mono.render_interleaved(&mut buf, 128);
    assert!(buf.iter().any(|s| *s != 0));
    // Eight-channel device, i32 interleaved: every channel carries the tone.
    let mut multi = OfflineRenderer::new(48_000, 8);
    multi.send(tone(-6.0, 440.0)).unwrap();
    let mut buf = vec![0i32; 8 * 1024];
    multi.render_interleaved(&mut buf, 256);
    for channel in 0..8 {
        assert!(buf.iter().skip(channel).step_by(8).any(|s| *s != 0));
    }
    // A device buffer that is not a whole number of frames: remainder is silence.
    let mut stereo = OfflineRenderer::new(48_000, 2);
    stereo.send(tone(-6.0, 440.0)).unwrap();
    let mut odd = vec![99u16; 9];
    stereo.core_mut().process_interleaved(&mut odd, 2);
    assert_eq!(odd[8], u16::SILENCE);
}

#[test]
fn integer_output_matches_float_render_and_never_wraps() {
    let mut a = OfflineRenderer::new(48_000, 1);
    let mut b = OfflineRenderer::new(48_000, 1);
    for e in [&mut a, &mut b] {
        e.send(tone(-6.0, 1234.0)).unwrap();
    }
    let float = a.render(4096, 256).remove(0);
    let mut ints = vec![0i16; 4096];
    b.render_interleaved(&mut ints, 256);
    for (f, i) in float.iter().zip(&ints) {
        assert!((f - f32::from(*i) / 32_767.0).abs() <= 1.0 / 32_767.0);
    }
}

#[test]
fn snapshot_and_telemetry_are_consistent_after_each_rate() {
    for rate in RATES {
        let mut engine = OfflineRenderer::new(rate, 2);
        engine.render(rate as usize, 512);
        let snap = engine.snapshot();
        assert_eq!(snap.sample_rate, rate);
        assert_eq!(snap.clock_frames, u64::from(rate));
        // Monitoring counters belong to the device layer, not the core.
        assert_eq!(snap.stats.callbacks, 0);
    }
}
