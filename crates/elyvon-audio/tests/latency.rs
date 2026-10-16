use elyvon_audio::{contracts::*, latency::*};
#[test]
fn parallel_graph_paths_align_and_delay_line_is_exact() {
    let graph = AudioGraphPlan {
        contract_version: AUDIO_CONTRACT_VERSION,
        nodes: vec![
            GraphNode {
                id: 1,
                input_channels: 0,
                output_channels: 1,
            },
            GraphNode {
                id: 2,
                input_channels: 1,
                output_channels: 1,
            },
            GraphNode {
                id: 3,
                input_channels: 1,
                output_channels: 1,
            },
        ],
        connections: vec![
            GraphConnection {
                from_node: 1,
                from_channel: 0,
                to_node: 3,
                to_channel: 0,
            },
            GraphConnection {
                from_node: 2,
                from_channel: 0,
                to_node: 3,
                to_channel: 0,
            },
        ],
    };
    let plan = LatencyPlan::compile(&graph, &[(1, 128), (2, 512), (3, 16)]).unwrap();
    assert_eq!(plan.output_frames, 528);
    assert_eq!(plan.edges[0].frames, 384);
    assert_eq!(plan.edges[1].frames, 0);
    let mut line = DelayLine::new(384).unwrap();
    for i in 0..385 {
        assert_eq!(
            line.process(if i == 0 { 1.0 } else { 0.0 }),
            if i == 384 { 1.0 } else { 0.0 }
        );
    }
    line.reset();
    assert_eq!(line.process(0.0), 0.0);
    assert!(LatencyPlan::compile(&graph, &[(99, 12)]).is_err());
}
#[test]
fn loopback_delay_measurement_finds_known_offset_and_rejects_ambiguous_data() {
    let mut state = 123u32;
    let reference: Vec<f32> = (0..256)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 16) as f32 / 65536.0 - 0.5
        })
        .collect();
    let mut captured = vec![0.0; 512];
    for (s, d) in reference.iter().zip(&mut captured[127..]) {
        *d = *s * 0.25;
    }
    let measurement = measure_loopback(&reference, &captured, 256).unwrap();
    assert_eq!(measurement.delay_frames, 127);
    assert!(measurement.correlation > 0.999);
    assert!(measure_loopback(&[0.0; 32], &[0.0; 64], 32).is_err());
    assert!(measure_loopback(&[1.0; 32], &[1.0; 64], 32).is_err());
}
