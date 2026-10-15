//! Control → real-time commands, carried by a wait-free SPSC ring buffer.
//!
//! The control thread owns a [`CommandSender`]; the audio callback owns the
//! matching [`CommandReceiver`]. Both sides are wait-free and allocation-free
//! after construction ([`rtrb`]), so the callback never blocks on the UI.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Default capacity: far more than a user can generate between two callbacks.
pub const COMMAND_QUEUE_CAPACITY: usize = 256;
/// Upper bound of commands applied per callback, so a flood cannot starve audio.
pub const MAX_COMMANDS_PER_BLOCK: usize = 64;

/// Lowest and highest test-tone levels accepted (dBFS). The ceiling keeps an
/// accidental click from producing a full-scale tone.
pub const TONE_MIN_DB: f32 = -60.0;
pub const TONE_MAX_DB: f32 = -6.0;
pub const TONE_DEFAULT_DB: f32 = -18.0;
pub const TONE_MIN_HZ: f32 = 20.0;
pub const TONE_MAX_HZ: f32 = 20_000.0;

/// Which output channels receive the test tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ToneChannels {
    All,
    /// Zero-based device channel.
    Only {
        channel: u16,
    },
}

/// Opt-in sine test tone used to verify an output device.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestTone {
    pub enabled: bool,
    pub frequency_hz: f32,
    pub level_db: f32,
    pub channels: ToneChannels,
}

impl Default for TestTone {
    fn default() -> Self {
        Self {
            enabled: false,
            frequency_hz: 440.0,
            level_db: TONE_DEFAULT_DB,
            channels: ToneChannels::All,
        }
    }
}

impl TestTone {
    /// Clamps frequency/level into the safe ranges (control thread only).
    pub fn sanitized(mut self, sample_rate: u32) -> Self {
        let nyquist_guard = (sample_rate as f32 * 0.45).max(TONE_MIN_HZ);
        self.frequency_hz = if self.frequency_hz.is_finite() {
            self.frequency_hz
                .clamp(TONE_MIN_HZ, TONE_MAX_HZ.min(nyquist_guard))
        } else {
            440.0
        };
        self.level_db = if self.level_db.is_finite() {
            self.level_db.clamp(TONE_MIN_DB, TONE_MAX_DB)
        } else {
            TONE_DEFAULT_DB
        };
        self
    }
}

/// A message to the audio thread. `Copy` and fixed-size: sending never allocates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Start the transport from its current position.
    Play,
    /// Stop the transport; the position is kept.
    Stop,
    /// Move the transport to an absolute frame.
    Locate { frame: u64 },
    /// Replace the test-tone settings (ramped, click-free).
    SetTestTone(TestTone),
}

/// The control-thread end of the command queue.
#[derive(Debug)]
pub struct CommandSender(rtrb::Producer<Command>);

/// The real-time end of the command queue.
#[derive(Debug)]
pub struct CommandReceiver(rtrb::Consumer<Command>);

/// Error returned when the audio thread is not draining commands fast enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

impl std::fmt::Display for QueueFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the audio engine command queue is full")
    }
}

impl std::error::Error for QueueFull {}

/// Allocates a queue. Control thread only.
pub fn command_queue(capacity: usize) -> (CommandSender, CommandReceiver) {
    let (tx, rx) = rtrb::RingBuffer::new(capacity.max(1));
    (CommandSender(tx), CommandReceiver(rx))
}

impl CommandSender {
    pub fn send(&mut self, command: Command) -> Result<(), QueueFull> {
        self.0.push(command).map_err(|_| QueueFull)
    }

    /// Free slots; useful for tests and back-pressure diagnostics.
    pub fn free_slots(&self) -> usize {
        self.0.slots()
    }
}

impl CommandReceiver {
    /// Wait-free pop. Real-time safe.
    #[inline]
    pub fn pop(&mut self) -> Option<Command> {
        self.0.pop().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_preserves_order_and_reports_full() {
        let (mut tx, mut rx) = command_queue(2);
        tx.send(Command::Play).unwrap();
        tx.send(Command::Locate { frame: 7 }).unwrap();
        assert_eq!(tx.send(Command::Stop), Err(QueueFull));
        assert_eq!(rx.pop(), Some(Command::Play));
        assert_eq!(rx.pop(), Some(Command::Locate { frame: 7 }));
        assert_eq!(rx.pop(), None);
        tx.send(Command::Stop).unwrap();
        assert_eq!(rx.pop(), Some(Command::Stop));
    }

    #[test]
    fn queue_is_lossless_across_threads() {
        let (mut tx, mut rx) = command_queue(16);
        let producer = std::thread::spawn(move || {
            for frame in 0..10_000u64 {
                while tx.send(Command::Locate { frame }).is_err() {
                    std::thread::yield_now();
                }
            }
        });
        let mut expected = 0u64;
        while expected < 10_000 {
            if let Some(Command::Locate { frame }) = rx.pop() {
                assert_eq!(frame, expected);
                expected += 1;
            }
        }
        producer.join().unwrap();
    }

    #[test]
    fn tone_is_clamped_to_safe_levels_and_below_nyquist() {
        let t = TestTone {
            enabled: true,
            frequency_hz: 30_000.0,
            level_db: 0.0,
            channels: ToneChannels::All,
        }
        .sanitized(44_100);
        assert_eq!(t.level_db, TONE_MAX_DB);
        assert!(t.frequency_hz < 22_050.0);
        let t = TestTone {
            frequency_hz: f32::NAN,
            level_db: f32::NEG_INFINITY,
            ..TestTone::default()
        }
        .sanitized(48_000);
        assert_eq!(t.frequency_hz, 440.0);
        assert_eq!(t.level_db, TONE_DEFAULT_DB);
    }
}
