//! Mixer and piano-roll commands that are not project edits: note
//! auditions, clip-indicator reset, graph status and stem export.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use elyvon_audio::media::MediaBuffer;
use elyvon_audio::render::{self, RenderRequest, StemsReport};
use elyvon_project::{MediaId, TrackId};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use ts_rs::TS;

use crate::error::{ErrorCode, IpcError, IpcResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MixerStatus {
    pub sample_rate: u32,
    /// Plug-in delay compensated on the longest path, in frames.
    pub latency_frames: u32,
}

/// Plays a note on the track's built-in instrument (instrument tracks) or
/// MIDI output (MIDI tracks) for `duration_ms`, without recording it.
#[tauri::command]
pub async fn midi_preview(
    app: AppHandle,
    track_id: TrackId,
    channel: u8,
    pitch: u8,
    velocity: u8,
    duration_ms: u32,
) -> IpcResult<()> {
    app.state::<AppState>().midi.preview(
        track_id,
        channel,
        pitch,
        velocity,
        Duration::from_millis(u64::from(duration_ms)),
    )
}

#[tauri::command]
pub async fn mixer_reset_clips(app: AppHandle) -> IpcResult<()> {
    app.state::<AppState>().audio.reset_clips();
    Ok(())
}

#[tauri::command]
pub async fn mixer_status(app: AppHandle) -> IpcResult<MixerStatus> {
    let audio = &app.state::<AppState>().audio;
    Ok(MixerStatus {
        sample_rate: audio.rate(),
        latency_frames: u32::try_from(audio.latency_frames()).unwrap_or(u32::MAX),
    })
}

/// Renders the master and each channel strip's post-fader output into
/// `directory` as 32-bit float WAV files (plus `stems.json`).
#[tauri::command]
pub async fn project_export_stems(app: AppHandle, directory: String) -> IpcResult<StemsReport> {
    let state = app.state::<AppState>();
    let project = {
        let session = state.session();
        state.ensure_editable()?;
        if state.audio.snapshot().is_some_and(|s| {
            matches!(
                s.state,
                elyvon_audio::transport::PlayState::Playing
                    | elyvon_audio::transport::PlayState::CountIn
            )
        }) {
            return Err(IpcError::new(
                ErrorCode::Busy,
                "Stop playback before exporting stems.",
            ));
        }
        state.rendering.store(true, Ordering::Release);
        session.project().clone()
    };
    struct RenderGuard(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for RenderGuard {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let guard = RenderGuard(Arc::clone(&state.rendering));
    let rate = state.audio.rate();
    let media: HashMap<MediaId, Arc<MediaBuffer>> = project
        .media
        .iter()
        .filter_map(|a| state.audio.buffer(a.id).map(|b| (a.id, b)))
        .collect();
    let dir = PathBuf::from(directory);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let lookup = |id| media.get(&id).cloned();
        render::export_stems(
            &dir,
            &project,
            &lookup,
            RenderRequest {
                sample_rate: rate,
                start_tick: 0,
                end_tick: None,
                block_frames: 1024,
                metronome: false,
            },
        )
        .map_err(|e| IpcError::new(ErrorCode::Io, e.to_string()))
    })
    .await
    .map_err(IpcError::from)?
}
