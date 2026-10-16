//! Shared, product-agnostic building blocks for Elyvon applications.
//!
//! This crate deliberately contains no UI, no Tauri and no audio-thread code.
//! Everything here is cheap to compile and safe to reuse from future Elyvon
//! products (each of which ships and updates independently).
//!
//! * [`music`] — exact integer musical time (ticks), tempo, metre and key.
//! * [`tempo`] — tempo/metre map: ticks ↔ seconds ↔ sample frames, tap tempo.
//! * [`grid`] — snap grid with triplets and swing.
//! * [`color`] — validated sRGB colours with a stable `#RRGGBB` encoding.
//! * [`fs`] — crash-safe atomic writes and size-limited reads.

pub mod color;
pub mod fs;
pub mod grid;
pub mod jobs;
pub mod music;
pub mod tempo;

pub use color::{Color, ColorParseError};
pub use music::{Key, KeyMode, MusicalPosition, PPQ, PitchClass, Tempo, Ticks, TimeSignature};
pub use tempo::{MeterChange, TapTempo, TempoChange, TempoMap};
