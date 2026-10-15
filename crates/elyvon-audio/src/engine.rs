//! Processor contracts for the future audio graph.
//!
//! The running engine core lives in [`crate::render`]; its status in [`crate::status`].

use serde::Serialize;
use ts_rs::TS;

/// Stream configuration negotiated with an audio device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub buffer_frames: u32,
    pub input_channels: u16,
    pub output_channels: u16,
}

/// Per-callback timing information.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessContext {
    pub sample_rate: u32,
    /// Frames in this block (≤ the configured buffer size).
    pub frames: usize,
    /// Song position of the first frame, in samples, if the transport runs.
    pub song_position_samples: Option<u64>,
}

/// Non-interleaved audio buffers for one block: one slice per channel.
#[derive(Debug)]
pub struct AudioBlock<'a, 'b> {
    pub inputs: &'a [&'b [f32]],
    pub outputs: &'a mut [&'b mut [f32]],
}

/// A node in the future processing graph (track, bus, insert effect, …).
///
/// * [`prepare`](Self::prepare) runs on a non-real-time thread before
///   processing starts or when the configuration changes. Allocate here.
/// * [`process`](Self::process) runs on the real-time audio thread. It must be
///   wait-free in practice: no allocation, no locks, no IO, no logging.
/// * [`reset`](Self::reset) clears internal state (e.g. after a transport jump)
///   and follows the same rules as `process`.
pub trait AudioProcessor: Send {
    fn prepare(&mut self, config: &AudioConfig);
    fn process(&mut self, ctx: &ProcessContext, block: &mut AudioBlock<'_, '_>);
    fn reset(&mut self);
}
