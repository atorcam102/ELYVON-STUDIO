//! Update policy shared by Elyvon applications.
//!
//! The Tauri updater plugin performs the network transfer and the mandatory
//! signature verification. This crate decides *what* may be installed:
//!
//! * [`version::decide`] — SemVer comparison, no downgrades, no silent move
//!   from stable to pre-release.
//! * [`platform`] — the supported OS/architecture matrix, installer kinds and
//!   the release artifact naming convention.
//! * [`manifest`] — strict validation of `latest.json`, exact per-platform
//!   selection (never another architecture), and manifest assembly for the
//!   release pipeline (`elyvon-release` binary).
//!
//! It has no Tauri dependency so it can be unit-tested in isolation and reused
//! by future Elyvon products, each of which updates independently.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod manifest;
pub mod platform;
pub mod version;

pub use manifest::{ManifestError, PlatformAsset, UpdateManifest};
pub use platform::{Arch, ArtifactKind, ArtifactNaming, Installer, Os, Platform, STUDIO_NAMING};
pub use version::{UpdateDecision, decide};
