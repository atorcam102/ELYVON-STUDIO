//! Frozen wire fixtures and invalid plans. No audio device is used.
use elyvon_audio::contracts::*;
use elyvon_audio::gain::{constant_power_pan, db_to_gain};

fn graph() -> AudioGraphPlan {
    serde_json::from_str(include_str!("fixtures/graph-v1.json")).expect("valid graph fixture")
}

#[test]
fn graph_v1_wire_fixture_is_stable_and_valid() {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/graph-v1.json")).expect("valid graph fixture");
    assert_eq!(serde_json::to_value(graph()).unwrap(), value);
    graph().validate().unwrap(); // parallel channel edges must not look cyclic
}

#[test]
fn graph_rejects_unknown_versions_duplicates_missing_nodes_channels_and_feedback() {
    let valid = graph();
    let mut bad = valid.clone();
    bad.contract_version = 2;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.nodes.push(bad.nodes[0].clone());
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.connections[0].to_node = 999;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.connections[0].from_channel = 2;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.connections.push(bad.connections[0]);
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.connections.push(GraphConnection {
        from_node: 2,
        from_channel: 0,
        to_node: 1,
        to_channel: 0,
    });
    assert!(bad.validate().is_err());
    let mut bad = valid;
    bad.connections[0].to_node = 1;
    assert!(bad.validate().is_err());
}

#[test]
fn midi_v1_preserves_wire_shape_and_callback_boundaries() {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/midi-v1.json")).unwrap();
    let events: Vec<MidiEvent> = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&events).unwrap(), value);
    for event in &events {
        event.validate(128).unwrap();
    }
    assert!(events[1].validate(127).is_err());
    assert!(events[0].validate(0).is_err());
    for (channel, pitch, velocity) in [(16, 60, 100), (0, 128, 100), (0, 60, 128)] {
        assert!(
            MidiEvent {
                frame_offset: 0,
                message: MidiMessage::NoteOn {
                    channel,
                    pitch,
                    velocity
                }
            }
            .validate(128)
            .is_err()
        );
    }
    MidiEvent {
        frame_offset: 0,
        message: MidiMessage::NoteOn {
            channel: 15,
            pitch: 127,
            velocity: 0,
        },
    }
    .validate(1)
    .unwrap();
}

#[test]
fn deterministic_signal_fixtures_verify_gain_and_pan_without_claiming_an_engine() {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Signal {
        name: String,
        samples: Vec<f64>,
        db: f64,
        pan: f64,
        output_energy: f64,
    }
    let signals: Vec<Signal> =
        serde_json::from_str(include_str!("fixtures/signals-v1.json")).unwrap();
    for signal in signals {
        let gain = db_to_gain(signal.db);
        let (l, r) = constant_power_pan(signal.pan);
        let energy: f64 = signal
            .samples
            .iter()
            .map(|s| (s * gain * l).powi(2) + (s * gain * r).powi(2))
            .sum();
        assert!(
            (energy - signal.output_energy).abs() < 1e-12,
            "{}",
            signal.name
        );
    }
}
