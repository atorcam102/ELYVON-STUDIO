//! Deterministic, driver-free rendering for CI and tests.
//!
//! [`OfflineRenderer`] drives the exact same [`EngineCore`] the device
//! callback uses, with callback-sized blocks, but without any audio hardware,
//! clock or thread. Identical inputs always give bit-identical outputs. The
//! [`analysis`] helpers measure the rendered signal (peak, RMS, frequency).

use std::sync::Arc;

use crate::command::{COMMAND_QUEUE_CAPACITY, Command, CommandSender, QueueFull, command_queue};
use crate::format::OutputSample;
use crate::render::{CoreConfig, EngineCore};
use crate::telemetry::{Telemetry, TelemetrySnapshot};

/// Headless engine harness. Not used by the desktop application at runtime.
#[derive(Debug)]
pub struct OfflineRenderer {
    core: EngineCore,
    sender: CommandSender,
    telemetry: Arc<Telemetry>,
    channels: usize,
}

impl OfflineRenderer {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let telemetry = Arc::new(Telemetry::new());
        telemetry.set_stream_format(sample_rate, channels, 0);
        let (sender, receiver) = command_queue(COMMAND_QUEUE_CAPACITY);
        let core = EngineCore::new(
            CoreConfig {
                sample_rate,
                output_channels: channels,
                start_clock: 0,
                start_transport: 0,
            },
            receiver,
            Arc::clone(&telemetry),
        );
        let channels = core.channels();
        Self {
            core,
            sender,
            telemetry,
            channels,
        }
    }

    pub fn send(&mut self, command: Command) -> Result<(), QueueFull> {
        self.sender.send(command)
    }

    pub fn core(&self) -> &EngineCore {
        &self.core
    }

    pub fn core_mut(&mut self) -> &mut EngineCore {
        &mut self.core
    }

    pub fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub fn snapshot(&self) -> TelemetrySnapshot {
        self.telemetry.snapshot()
    }

    /// Renders `frames` frames in callbacks of `block_frames` (the last one
    /// may be shorter) and returns planar channels. Allocates the result.
    pub fn render(&mut self, frames: usize, block_frames: usize) -> Vec<Vec<f32>> {
        let block = block_frames.max(1);
        let mut channels = vec![vec![0.0f32; frames]; self.channels];
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
            let mut views: Vec<&mut [f32]> = channels
                .iter_mut()
                .map(|c| &mut c[done..done + n])
                .collect();
            self.core.process_planar(&mut views);
            done += n;
        }
        channels
    }

    /// Renders into an interleaved device-format buffer in `block_frames`
    /// callbacks, exactly like a device stream would.
    pub fn render_interleaved<S: OutputSample>(&mut self, out: &mut [S], block_frames: usize) {
        let block = block_frames.max(1) * self.channels;
        for chunk in out.chunks_mut(block) {
            self.core.process_interleaved(chunk, self.channels);
        }
    }
}

/// Signal measurements used by tests and diagnostics.
pub mod analysis {
    /// Largest absolute sample value.
    pub fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |p, v| p.max(v.abs()))
    }

    /// Root-mean-square level.
    pub fn rms(samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().map(|v| f64::from(*v).powi(2)).sum();
        (sum / samples.len() as f64).sqrt()
    }

    /// Frequency estimate from rising zero crossings with linear
    /// interpolation between samples. Accurate for clean periodic tones.
    pub fn estimate_frequency(samples: &[f32], sample_rate: u32) -> Option<f64> {
        let mut crossings = Vec::new();
        for i in 1..samples.len() {
            let (a, b) = (f64::from(samples[i - 1]), f64::from(samples[i]));
            if a < 0.0 && b >= 0.0 {
                crossings.push((i - 1) as f64 + a / (a - b));
            }
        }
        if crossings.len() < 2 {
            return None;
        }
        let first = crossings[0];
        let last = crossings[crossings.len() - 1];
        let periods = (crossings.len() - 1) as f64;
        Some(f64::from(sample_rate) * periods / (last - first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_helpers() {
        assert_eq!(analysis::peak(&[0.1, -0.7, 0.3]), 0.7);
        assert!((analysis::rms(&[1.0, -1.0]) - 1.0).abs() < 1e-12);
        assert_eq!(analysis::rms(&[]), 0.0);
        assert_eq!(analysis::estimate_frequency(&[0.0; 64], 48_000), None);
    }
}
