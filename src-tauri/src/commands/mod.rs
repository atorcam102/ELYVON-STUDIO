//! IPC command handlers, grouped by area. All commands are `async` so they run
//! on Tauri's worker pool, never on the main (UI event loop) thread.

pub mod audio;
pub mod browser;
pub mod project;
pub mod system;
