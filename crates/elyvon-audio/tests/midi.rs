#![allow(clippy::unwrap_used)]
use elyvon_audio::{
    contracts::MidiEvent,
    engine::{self, MidiSink},
    midi::{Packet, smf},
    plan::build_plan,
    transport::TransportCommand,
};
use elyvon_project::{
    EditCommand, Project,
    io::{MidiImportRegion, StoredMidiEvent},
};
fn project() -> Project {
    let mut p = Project::new("MIDI contract");
    let events = vec![
        (0, vec![0xc2, 10]),
        (0, vec![0xb2, 64, 127]),
        (0, vec![0x92, 60, 100]),
        (240, vec![0xe2, 0, 65]),
        (480, vec![0xa2, 60, 70]),
        (600, vec![0xd2, 80]),
        (960, vec![0x82, 60, 20]),
        (1200, vec![0xb2, 64, 0]),
        (1440, vec![0xf0, 0x7d, 1, 2, 0xf7]),
        (1920, vec![0xf8]),
    ]
    .into_iter()
    .map(|(tick, bytes)| StoredMidiEvent { tick, bytes })
    .collect::<Vec<_>>();
    let notes = smf::notes_from_events(&events, 2000);
    elyvon_project::edit::apply(
        &mut p,
        &EditCommand::ImportMidi {
            regions: vec![MidiImportRegion {
                name: "Keys".into(),
                track_id: None,
                start: 0,
                length: 2000,
                notes,
                events,
            }],
            tempo: None,
            tempo_changes: vec![],
            time_signature: None,
            meter_changes: vec![],
        },
    )
    .unwrap();
    p.transport.tempo_changes = vec![elyvon_core::TempoChange {
        tick: 960,
        bpm: elyvon_core::Tempo::new(90.0).unwrap(),
    }];
    p
}
#[test]
fn smf_type_one_roundtrip_preserves_channels_controls_sysex_and_tempo() {
    let p = project();
    let file = smf::encode(&p).unwrap();
    let parsed = smf::decode(&file).unwrap();
    assert_eq!(parsed.regions.len(), 1);
    assert_eq!(parsed.regions[0].events, p.tracks[0].regions[0].midi_events);
    assert_eq!(parsed.tempo, p.transport.tempo);
    assert_eq!(parsed.tempo_changes.len(), p.transport.tempo_changes.len());
    for (a, b) in parsed.tempo_changes.iter().zip(&p.transport.tempo_changes) {
        assert_eq!(a.tick, b.tick);
        assert!(
            (a.bpm.bpm() - b.bpm.bpm()).abs() < 0.0001,
            "SMF quantizes tempo to integer microseconds per quarter"
        );
    }
    let mut reopened = Project::new("Reopened");
    elyvon_project::edit::apply(
        &mut reopened,
        &EditCommand::ImportMidi {
            regions: parsed.regions,
            tempo: Some(parsed.tempo),
            tempo_changes: parsed.tempo_changes,
            time_signature: Some(parsed.time_signature),
            meter_changes: parsed.meter_changes,
        },
    )
    .unwrap();
    let parsed_again = smf::decode(&smf::encode(&reopened).unwrap()).unwrap();
    assert_eq!(
        parsed_again.regions[0].events,
        p.tracks[0].regions[0].midi_events
    );
}
#[test]
fn smf_type_zero_scales_ppq_and_invalid_data_fails_explicitly() {
    let data = [
        0, 0xff, 0x51, 3, 7, 0xa1, 0x20, 0, 0x90, 60, 100, 0x83, 0x60, 0x80, 60, 0, 0, 0xff, 0x2f,
        0,
    ];
    let mut bytes = b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk".to_vec();
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend(data);
    let file = smf::decode(&bytes).unwrap();
    assert_eq!(file.regions[0].notes[0].length, 960);
    assert!(smf::decode(&bytes[..bytes.len() - 4]).is_err());
    assert!(smf::decode(b"not midi").is_err());
    assert!(Packet::parse(&[0x90, 128, 1]).is_none());
    assert!(Packet::parse(&[0xe0, 0, 128]).is_none());
    let mut big = vec![0xf0];
    big.extend([1; 255]);
    big.push(0xf7);
    assert!(Packet::parse(&big).is_none());
    for bytes in [
        &[0xb0, 64, 127][..],
        &[0xc0, 2],
        &[0xd0, 1],
        &[0xe0, 127, 127],
        &[0xf8],
        &[0xf0, 0x7d, 0xf7],
    ] {
        assert!(Packet::parse(bytes).is_some());
    }
}
#[derive(Default)]
struct Log {
    base: u64,
    frames: u32,
    events: Vec<(u64, Vec<u8>)>,
    resets: Vec<u64>,
}
impl MidiSink for Log {
    fn event(&mut self, _: usize, e: MidiEvent) {
        assert!(e.frame_offset < self.frames);
        self.events.push((
            self.base + u64::from(e.frame_offset),
            Packet::from_message(e.message).unwrap().bytes().to_vec(),
        ));
    }
    fn packet(&mut self, _: usize, frame: u32, p: Packet) {
        assert!(frame < self.frames);
        self.events
            .push((self.base + u64::from(frame), p.bytes().to_vec()));
    }
    fn reset(&mut self, frame: u32) {
        assert!(frame < self.frames);
        self.resets.push(self.base + u64::from(frame));
    }
}
fn schedule(block: usize) -> Vec<(u64, Vec<u8>)> {
    let p = project();
    let plan = build_plan(&p, &|_| None, 48000, 1);
    let (mut handle, mut engine) = engine::create(48000, block, plan);
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut log = Log::default();
    let mut out = vec![0.0; block * 2];
    let mut frame = 0usize;
    while frame < 100000 {
        let n = block.min(100000 - frame);
        log.base = frame as u64;
        log.frames = n as u32;
        engine.process(&mut out[..n * 2], 2, &mut log);
        assert!(!handle.drain_midi().1);
        frame += n;
    }
    handle.send(TransportCommand::Stop);
    log.base = frame as u64;
    log.frames = block as u32;
    engine.process(&mut out, 2, &mut log);
    assert!(log.resets.contains(&(frame as u64)));
    log.events
}
#[test]
fn raw_midi_events_are_sample_accurate_for_all_block_sizes() {
    let reference = schedule(512);
    for block in [1, 7, 64, 4096] {
        assert_eq!(schedule(block), reference);
    }
}
#[test]
fn sustain_and_held_notes_are_closed_at_boundaries_and_crops_chase_state() {
    let mut events = vec![
        StoredMidiEvent {
            tick: 0,
            bytes: vec![0xb3, 64, 127],
        },
        StoredMidiEvent {
            tick: 10,
            bytes: vec![0x93, 60, 90],
        },
    ];
    smf::close_sequence(&mut events, 100);
    assert!(events.iter().any(|e| e.bytes == [0xb3, 64, 0]));
    assert!(events.iter().any(|e| e.bytes == [0x83, 60, 0]));
    let before = events.clone();
    smf::close_sequence(&mut events, 100);
    assert_eq!(events, before, "closing is idempotent");
    let cropped = elyvon_project::io::crop_midi(&events, 50, 80);
    assert!(
        cropped
            .iter()
            .any(|e| e.tick == 0 && e.bytes == [0x93, 60, 90])
    );
    assert!(
        cropped
            .iter()
            .any(|e| e.tick == 30 && e.bytes == [0x83, 60, 0])
    );
}
#[test]
fn cycle_panic_at_callback_boundary_is_delivered_in_the_next_valid_callback() {
    let mut p = project();
    p.transport.loop_range.enabled = true;
    p.transport.loop_range.start = 0;
    p.transport.loop_range.end = 960;
    p.transport.tempo_changes.clear();
    let plan = build_plan(&p, &|_| None, 48000, 1);
    let (mut handle, mut engine) = engine::create(48000, 24000, plan);
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut log = Log {
        frames: 24000,
        ..Log::default()
    };
    let mut out = vec![0.0; 48000];
    engine.process(&mut out, 2, &mut log);
    log.base = 24000;
    engine.process(&mut out, 2, &mut log);
    assert!(log.resets.contains(&24000));
    handle.send(TransportCommand::Locate { tick: 10 });
    log.base = 48000;
    engine.process(&mut out, 2, &mut log);
    assert!(log.resets.contains(&48000));
}
