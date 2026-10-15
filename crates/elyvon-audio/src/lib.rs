//! Elyvon Studio audio engine core — driver-free and deterministic.
//!
//! This crate contains everything the real-time audio callback executes, but
//! no device code: devices live in `elyvon-audio-device`, which moves an
//! [`render::EngineCore`] into its callback. Because the core has no driver,
//! CI renders it headlessly ([`offline`]) with bit-identical results.
//!
//! * [`render`] — the real-time core: transport, sample clock, silence,
//!   opt-in test tone, meters, format conversion.
//! * [`command`] — wait-free SPSC command queue (control → audio thread).
//! * [`telemetry`] — lock-free meters, clocks and xrun/dropout counters
//!   (audio thread → control thread).
//! * [`format`] — planar/interleaved and sample-format conversion with clipping.
//! * [`status`] — engine status reported to the UI.
//! * [`offline`] — headless renderer and signal analysis for tests.
//! * [`engine`], [`contracts`], [`gain`], [`rt`] — processor/graph/MIDI
//!   contracts and primitives for later phases.
//!
//! # Real-time rules
//!
//! Code reachable from the audio callback must never allocate or free memory,
//! take a lock that a non-real-time thread can hold, perform file or network
//! IO, log, or call into the UI. Parameters reach the callback through the
//! command queue or atomics; telemetry leaves it through atomics only.
//! `tests/realtime.rs` verifies the no-allocation rule. See
//! `docs/AUDIO_ENGINE.md`.

pub mod command;
pub mod contracts;
pub mod engine;
pub mod format;
pub mod gain;
pub mod offline;
pub mod render;
pub mod rt;
pub mod status;
pub mod telemetry;
