//! Version 1 control-thread contracts, retained for compatibility. Native MIDI
//! ports use the bounded packet bridge; a hosted graph executor is future work.
//! Deserialisation is structural; callers MUST validate before using a plan.
//! Vec allocation and validation belong off the real-time thread.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use ts_rs::TS;

pub const AUDIO_CONTRACT_VERSION: u32 = 1;

/// MIDI 1.0 channel messages; channels are zero-based, data bytes are 0..=127.
/// SysEx, MIDI 2.0 and device routing are outside this version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum MidiMessage {
    ProgramChange {
        channel: u8,
        program: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
    ChannelPressure {
        channel: u8,
        pressure: u8,
    },
    PolyPressure {
        channel: u8,
        pitch: u8,
        pressure: u8,
    },
    NoteOn {
        channel: u8,
        pitch: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        pitch: u8,
        velocity: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
}

/// Offset from the first sample of ONE callback, independent of project ticks.
/// Events must be ordered by frame_offset; conversion/scheduling is not implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiEvent {
    pub frame_offset: u32,
    pub message: MidiMessage,
}

impl MidiEvent {
    pub fn validate(&self, block_frames: u32) -> Result<(), &'static str> {
        if self.frame_offset >= block_frames {
            return Err("MIDI frame offset is outside the callback block");
        }
        if crate::midi::Packet::from_message(self.message).is_none() {
            return Err("invalid MIDI 1.0 channel or data byte");
        }
        // A NoteOn with velocity zero is legal MIDI (interpreted as NoteOff).
        Ok(())
    }
}

/// Descriptive control-thread node, NOT an AudioProcessor instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphNode {
    pub id: u32,
    pub input_channels: u16,
    pub output_channels: u16,
}

/// One channel-to-channel connection (zero-based channel indices).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphConnection {
    pub from_node: u32,
    pub from_channel: u16,
    pub to_node: u32,
    pub to_channel: u16,
}

/// A bounded, acyclic routing proposal. Not saved into schema-1 projects and
/// not executable. Fan-in is descriptive; there is no mixer/latency compensation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioGraphPlan {
    pub contract_version: u32,
    pub nodes: Vec<GraphNode>,
    pub connections: Vec<GraphConnection>,
}

impl AudioGraphPlan {
    /// Control-thread only: allocates maps/queues; NEVER call in process/reset.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contract_version != AUDIO_CONTRACT_VERSION {
            return Err("unsupported audio graph contract version");
        }
        if self.nodes.len() > 4096 || self.connections.len() > 65536 {
            return Err("audio graph exceeds control-thread limits");
        }
        let mut nodes = HashMap::new();
        for node in &self.nodes {
            if node.input_channels > 256 || node.output_channels > 256 {
                return Err("audio graph node exceeds channel limit");
            }
            if nodes.insert(node.id, node).is_some() {
                return Err("duplicate audio graph node id");
            }
        }
        let mut seen = HashSet::new();
        let mut indegree: HashMap<u32, usize> = nodes.keys().map(|id| (*id, 0)).collect();
        let mut outgoing: HashMap<u32, Vec<u32>> = HashMap::new();
        for edge in &self.connections {
            let from = nodes.get(&edge.from_node).ok_or("missing source node")?;
            let to = nodes.get(&edge.to_node).ok_or("missing destination node")?;
            if edge.from_channel >= from.output_channels || edge.to_channel >= to.input_channels {
                return Err("audio graph channel is out of range");
            }
            if !seen.insert(*edge) {
                return Err("duplicate audio graph connection");
            }
            *indegree
                .get_mut(&edge.to_node)
                .ok_or("missing destination node")? += 1;
            outgoing
                .entry(edge.from_node)
                .or_default()
                .push(edge.to_node);
        }
        let mut ready: Vec<u32> = indegree
            .iter()
            .filter_map(|(id, n)| (*n == 0).then_some(*id))
            .collect();
        let mut visited = 0;
        while let Some(id) = ready.pop() {
            visited += 1;
            if let Some(destinations) = outgoing.get(&id) {
                for destination in destinations {
                    let n = indegree
                        .get_mut(destination)
                        .ok_or("missing destination node")?;
                    *n -= 1;
                    if *n == 0 {
                        ready.push(*destination);
                    }
                }
            }
        }
        if visited != nodes.len() {
            return Err("audio graph contains a feedback cycle");
        }
        Ok(())
    }
}
