//! Hardware validation tool: lists devices and plays the test tone through
//! the production device manager, printing real telemetry every second.
//!
//! ```text
//! cargo run --release -p elyvon-audio-device --example audio-probe -- --list
//! cargo run --release -p elyvon-audio-device --example audio-probe -- \
//!     [--host ID] [--device ID] [--rate HZ] [--buffer FRAMES] [--seconds N] \
//!     [--level DB] [--channel N] [--json FILE]
//! ```
//!
//! Without `--device` the system default output is used. The tone is
//! −18 dBFS by default (never above −6 dBFS). Unplug the device while it
//! runs to exercise hot-unplug handling.

use std::time::{Duration, Instant};

use elyvon_audio::command::{TestTone, ToneChannels};
use elyvon_audio::status::EngineState;
use elyvon_audio_device::SystemBackend;
use elyvon_audio_device::backend::Backend;
use elyvon_audio_device::manager::{AudioManager, ManagerOptions};
use elyvon_audio_device::types::{AudioPreferences, AudioSetup};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let list = SystemBackend::default().list();
    println!("== Hosts");
    for h in &list.hosts {
        println!(
            "  {} ({}){}",
            h.id,
            h.name,
            if h.is_default { " [default]" } else { "" }
        );
    }
    for (title, devices) in [("Outputs", &list.outputs), ("Inputs", &list.inputs)] {
        println!("== {title}");
        for d in devices {
            println!(
                "  {}{}\n     id={} max_ch={} rates={:?} buffers={:?}",
                d.name,
                if d.is_default { " [default]" } else { "" },
                d.id,
                d.max_channels,
                d.sample_rates,
                d.buffer_sizes
            );
            if let Some(c) = &d.default_config {
                println!(
                    "     default: {} ch, {} Hz, {} ({} bit)",
                    c.channels, c.sample_rate, c.sample_format, c.bits_per_sample
                );
            }
            for r in &d.ranges {
                println!(
                    "     range: {} ch {}-{} Hz {} buffers {:?}-{:?}{}",
                    r.channels,
                    r.min_sample_rate,
                    r.max_sample_rate,
                    r.sample_format,
                    r.min_buffer_frames,
                    r.max_buffer_frames,
                    if r.supported_by_engine {
                        ""
                    } else {
                        " (unsupported format)"
                    }
                );
            }
            if let Some(e) = &d.error {
                println!("     error: {e}");
            }
        }
    }
    for e in &list.errors {
        println!("  host error: {e}");
    }
    if args.iter().any(|a| a == "--list") {
        return;
    }

    let mut prefs = AudioPreferences::default();
    prefs.apply_setup(AudioSetup {
        enabled: true,
        host: arg(&args, "--host"),
        output_device: arg(&args, "--device"),
        output_device_name: None,
        input_device: arg(&args, "--input"),
        sample_rate: arg(&args, "--rate").and_then(|v| v.parse().ok()),
        buffer_frames: arg(&args, "--buffer").and_then(|v| v.parse().ok()),
    });
    let seconds: u64 = arg(&args, "--seconds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let level: f32 = arg(&args, "--level")
        .and_then(|v| v.parse().ok())
        .unwrap_or(-18.0);
    let channels = match arg(&args, "--channel").and_then(|v| v.parse().ok()) {
        Some(channel) => ToneChannels::Only { channel },
        None => ToneChannels::All,
    };
    let manager = AudioManager::start(
        Box::new(SystemBackend::default()),
        prefs,
        ManagerOptions::default(),
        Some(Box::new(|s| {
            println!(
                "status: {:?} {}",
                s.state,
                s.message.clone().unwrap_or_default()
            );
            for n in &s.notices {
                println!("  notice {:?}: {}", n.code, n.message);
            }
        })),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while manager.status().state == EngineState::Starting && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = manager.status();
    println!("== Stream\n{status:#?}");
    if status.state != EngineState::Running {
        std::process::exit(2);
    }
    let tone = manager
        .set_test_tone(TestTone {
            enabled: true,
            frequency_hz: 440.0,
            level_db: level,
            channels,
        })
        .expect("engine running");
    manager.play().expect("engine running");
    println!("== Playing {tone:?} for {seconds} s");
    let mut samples = Vec::new();
    for _ in 0..seconds {
        std::thread::sleep(Duration::from_secs(1));
        let t = manager.telemetry();
        println!(
            "t={:.2}s clock={} peaks={:?} in={:?} callbacks={} late={} gaps={} xruns={} errors={} load={:.1}% block={} latency={:?}ms state={:?}",
            t.transport_frames as f64 / f64::from(t.sample_rate.max(1)),
            t.clock_frames,
            t.output_peak_db,
            t.input_peak_db,
            t.stats.callbacks,
            t.stats.late_callbacks,
            t.stats.timing_gaps,
            t.stats.backend_xruns,
            t.stats.stream_errors,
            t.stats.max_callback_load * 100.0,
            t.stats.last_block_frames,
            t.stats.measured_output_latency_ms,
            manager.status().state,
        );
        samples.push(t);
    }
    let _ = manager.set_test_tone(TestTone::default());
    let _ = manager.stop();
    std::thread::sleep(Duration::from_millis(100));
    if let Some(path) = arg(&args, "--json") {
        let report = serde_json::json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "status": manager.status(),
            "samples": samples,
        });
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&report).expect("serialisable"),
        )
        .expect("write report");
        println!("report written to {path}");
    }
}
