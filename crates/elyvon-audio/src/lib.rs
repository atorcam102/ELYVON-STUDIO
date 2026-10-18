//! The Elyvon Studio audio engine.
//!
//! * [`media`] — decoding (WAV/AIFF/FLAC/MP3/Ogg Vorbis), sample-rate
//!   conversion and waveform peaks, on worker threads.
//! * [`plan`] — an immutable playback plan built from a project on the control
//!   thread: sample-accurate clip segments, MIDI notes, tempo map, metronome.
//! * [`engine`] — engine status, the real-time [`engine::Engine`] and its
//!   lock-free control handle.
//! * [`transport`] — transport commands and the state shared with the UI.
//! * [`fx`] — native effects (equaliser, dynamics, delay, reverbs,
//!   modulation, distortion, pitch) used as mixer inserts.
//! * [`render`] — offline (faster than real time) rendering and WAV export
//!   through the same engine code path.
//! * [`device`] — audio-device output (feature `device`) and an internal clock.
//! * [`rt`], [`gain`], [`contracts`] — primitives and versioned contracts.
//!
//! # Real-time rules
//!
//! Code reachable from the audio callback must never allocate or free memory,
//! take a lock that a non-real-time thread can hold, perform file or network
//! IO, log, or call into the UI. Plans and commands reach the callback through
//! pre-allocated lock-free SPSC queues (`rtrb`); replaced plans travel back to
//! the control thread to be freed there. See `docs/ARCHITECTURE.md`.

pub mod analysis;
pub mod automation;
pub mod contracts;
pub mod device;
pub mod dsp;
pub mod engine;
pub mod fx;
pub mod gain;
#[cfg(feature = "device")]
pub mod input;
pub mod latency;
pub mod media;
pub mod midi;
pub mod plan;
pub mod recording;
pub mod render;
pub mod rt;
pub mod spectrum;
pub mod synth;
pub mod transport;
pub mod waveform;
