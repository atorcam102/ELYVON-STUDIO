//! Contracts for the future Elyvon Studio audio engine.
//!
//! **There is no audio engine in Phase 1.** This crate fixes the boundaries the
//! engine will be built against so the rest of the application can already be
//! structured correctly:
//!
//! * [`engine`] — engine status reporting and the [`engine::AudioProcessor`]
//!   contract every real-time node will implement.
//! * [`rt`] — lock-free primitives for passing values to the audio thread.
//! * [`gain`] — the gain and pan laws the mixer will apply.
//!
//! # Real-time rules
//!
//! Code reachable from the audio callback must never allocate or free memory,
//! take a lock that a non-real-time thread can hold, perform file or network
//! IO, log, or call into the UI. Parameters reach the callback through atomics
//! ([`rt::AtomicF32`]) or pre-allocated lock-free queues; anything that might
//! block happens on a separate worker thread. See `docs/ARCHITECTURE.md`.

pub mod contracts;
pub mod engine;
pub mod gain;
pub mod rt;
