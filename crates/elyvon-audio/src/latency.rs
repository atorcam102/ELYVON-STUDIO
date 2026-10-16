//! Control-thread latency planning and RT delay lines. The current mixer has
//! no hosted plug-ins; this makes graph compensation explicit without claiming
//! that a plug-in executor or direct hardware monitoring exists.
use crate::contracts::AudioGraphPlan;
use std::collections::{HashMap, VecDeque};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeDelay {
    pub connection: usize,
    pub frames: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatencyPlan {
    pub edges: Vec<EdgeDelay>,
    pub output_frames: u32,
}
impl LatencyPlan {
    pub fn compile(graph: &AudioGraphPlan, node_delays: &[(u32, u32)]) -> Result<Self, String> {
        graph.validate().map_err(str::to_owned)?;
        let mut own = HashMap::new();
        for &(id, delay) in node_delays {
            if own.insert(id, delay).is_some() || !graph.nodes.iter().any(|n| n.id == id) {
                return Err("unknown or duplicate latency node".into());
            }
        }
        let mut indegree: HashMap<_, usize> = graph.nodes.iter().map(|n| (n.id, 0)).collect();
        let mut incoming = HashMap::<u32, u32>::new();
        let mut done = HashMap::<u32, u32>::new();
        for e in &graph.connections {
            *indegree.get_mut(&e.to_node).ok_or("missing node")? += 1;
        }
        let mut queue: VecDeque<_> = indegree
            .iter()
            .filter_map(|(&id, &n)| (n == 0).then_some(id))
            .collect();
        while let Some(id) = queue.pop_front() {
            let latency = incoming
                .get(&id)
                .copied()
                .unwrap_or(0)
                .checked_add(own.get(&id).copied().unwrap_or(0))
                .ok_or("graph latency overflow")?;
            done.insert(id, latency);
            for e in graph.connections.iter().filter(|e| e.from_node == id) {
                let at = incoming.entry(e.to_node).or_default();
                *at = (*at).max(latency);
                let n = indegree.get_mut(&e.to_node).ok_or("missing node")?;
                *n -= 1;
                if *n == 0 {
                    queue.push_back(e.to_node);
                }
            }
        }
        let edges = graph
            .connections
            .iter()
            .enumerate()
            .map(|(connection, e)| EdgeDelay {
                connection,
                frames: incoming
                    .get(&e.to_node)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(done.get(&e.from_node).copied().unwrap_or(0)),
            })
            .collect();
        Ok(Self {
            edges,
            output_frames: done.values().copied().max().unwrap_or(0),
        })
    }
}
#[derive(Debug)]
pub struct DelayLine {
    buffer: Vec<f32>,
    at: usize,
}
impl DelayLine {
    pub fn new(frames: usize) -> Result<Self, &'static str> {
        if frames > 7680000 {
            return Err("delay exceeds ten seconds at the maximum media rate");
        }
        Ok(Self {
            buffer: vec![0.0; frames],
            at: 0,
        })
    }
    pub fn process(&mut self, input: f32) -> f32 {
        if self.buffer.is_empty() {
            return input;
        }
        let out = self.buffer[self.at];
        self.buffer[self.at] = input;
        self.at = (self.at + 1) % self.buffer.len();
        out
    }
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.at = 0;
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopbackMeasurement {
    pub delay_frames: u32,
    pub correlation: f64,
}
/// Cross-correlate a short, nonperiodic calibration signal with a recorded
/// loopback, off RT. Ambiguous/quiet measurements are rejected.
pub fn measure_loopback(
    reference: &[f32],
    captured: &[f32],
    max_delay: usize,
) -> Result<LoopbackMeasurement, &'static str> {
    if reference.len() < 16
        || reference.len() > 8192
        || max_delay > 192000
        || captured.len() < reference.len() + max_delay
        || reference.iter().chain(captured).any(|s| !s.is_finite())
    {
        return Err("invalid calibration signal or search range");
    }
    let energy = reference.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
    if energy < 1e-8 {
        return Err("calibration reference is silent");
    }
    let mut best = (0usize, -1.0f64);
    let mut second = -1.0f64;
    for delay in 0..=max_delay {
        let segment = &captured[delay..delay + reference.len()];
        let e = segment.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
        let dot = reference
            .iter()
            .zip(segment)
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum::<f64>();
        let c = if e > 1e-12 {
            dot / (energy * e).sqrt()
        } else {
            0.0
        };
        if c > best.1 {
            second = best.1;
            best = (delay, c);
        } else {
            second = second.max(c);
        }
    }
    if best.1 < 0.8 || best.1 - second < 0.01 {
        return Err("loopback measurement is weak or ambiguous; use a nonperiodic test signal");
    }
    Ok(LoopbackMeasurement {
        delay_frames: best.0 as u32,
        correlation: best.1,
    })
}
