//! Manual hardware evidence (ignored by default): enumerates the audio
//! output/input devices and MIDI ports the native backends see on this
//! machine, and negotiates (opens and closes) the default output. It never
//! records from an input. Run with:
//!
//! `cargo test -p elyvon-audio --features device --test hardware_probe -- --ignored --nocapture`
#![cfg(feature = "device")]

#[test]
#[ignore = "touches real devices; run manually"]
fn enumerate_devices_and_ports() {
    let outputs = elyvon_audio::device::output_devices();
    let inputs = elyvon_audio::input::input_devices();
    let midi = elyvon_audio::midi::ports::enumerate();
    println!(
        "platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    for d in &outputs {
        println!(
            "audio out: {} ({}){}",
            d.name,
            d.id,
            if d.is_default { " [default]" } else { "" }
        );
    }
    for d in &inputs {
        println!(
            "audio in:  {} ({}){}",
            d.name,
            d.id,
            if d.is_default { " [default]" } else { "" }
        );
    }
    for p in &midi.inputs {
        println!("midi in:   {} ({})", p.name, p.id);
    }
    for p in &midi.outputs {
        println!("midi out:  {} ({})", p.name, p.id);
    }
    println!(
        "midi error: {:?}; virtual ports: {}",
        midi.error, midi.virtual_ports
    );
    match elyvon_audio::device::negotiate(None, 48_000, None) {
        Ok((n, format)) => println!(
            "default output negotiated: {} {} Hz, buffer {} frames, {} ch, {format:?}",
            n.device_name, n.config.sample_rate, n.config.buffer_frames, n.config.output_channels
        ),
        Err(e) => println!("default output unavailable: {e}"),
    }
}
