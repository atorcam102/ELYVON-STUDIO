//! Audio engine commands: devices, transport, media import/relink/peaks and
//! offline render export.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use elyvon_audio::device::{self, DeviceInfo};
use elyvon_audio::engine::EngineStatus;
use elyvon_audio::media::{MediaBuffer, MediaStatus, MediaStore, SUPPORTED_EXTENSIONS};
use elyvon_audio::render::{self, RenderReport, RenderRequest};
use elyvon_audio::transport::{TransportCommand, TransportSnapshot};
use elyvon_audio::waveform;
use elyvon_core::Ticks;
use elyvon_project::{
    AudioPlacement, EditCommand, EditOutcome, MediaAsset, MediaId, PlacementTrack, RegionContent,
    RegionId, SessionView, TrackId,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Manager, State};
use ts_rs::TS;

use crate::error::{ErrorCode, IpcError, IpcResult};
use crate::state::AppState;

fn unavailable() -> IpcError {
    IpcError::new(
        ErrorCode::AudioUnavailable,
        "The audio engine is not running.",
    )
}

#[tauri::command]
pub async fn audio_devices() -> IpcResult<Vec<DeviceInfo>> {
    tauri::async_runtime::spawn_blocking(device::output_devices)
        .await
        .map_err(IpcError::from)
}

/// Reopens the output with the current preferences.
#[tauri::command]
pub async fn audio_restart(app: AppHandle) -> IpcResult<EngineStatus> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        state.ensure_editable()?;
        let settings = state.settings().audio.clone();
        let rate = state.session().project().sample_rate;
        state.audio.start(&handle, &settings, rate);
        Ok(state.audio.status())
    })
    .await
    .map_err(IpcError::from)?
}

#[tauri::command]
pub async fn transport_command(
    app: AppHandle,
    command: TransportCommand,
) -> IpcResult<TransportSnapshot> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if matches!(command, TransportCommand::Stop) && state.capture.is_active() {
            state.capture.stop(&app, false)?;
            return state.audio.snapshot().ok_or_else(unavailable);
        }
        if state.capture.status().monitoring {
            state.midi.ensure_idle()?;
        } else {
            state.ensure_editable()?;
        }
        state.audio.command(command).ok_or_else(unavailable)
    })
    .await
    .map_err(IpcError::from)?
}

#[tauri::command]
pub async fn transport_state(state: State<'_, AppState>) -> IpcResult<Option<TransportSnapshot>> {
    Ok(state.audio.snapshot())
}

/// Tap tempo measured with the native monotonic clock.
#[tauri::command]
pub async fn transport_tap(state: State<'_, AppState>) -> IpcResult<Option<f64>> {
    Ok(state.audio.tap())
}

#[tauri::command]
pub async fn media_formats() -> IpcResult<Vec<String>> {
    Ok(SUPPORTED_EXTENSIONS
        .iter()
        .map(|s| (*s).to_owned())
        .collect())
}

#[tauri::command]
pub async fn media_status(app: AppHandle) -> IpcResult<Vec<MediaStatus>> {
    Ok(app.state::<AppState>().audio.media_statuses(&app))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ImportLayout {
    /// One new audio track per file, all starting at `start`.
    NewTracks,
    /// Files one after another on `track_id` (or a new track).
    Sequential,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportFailure {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportResult {
    pub view: SessionView,
    pub outcome: EditOutcome,
    pub failures: Vec<ImportFailure>,
}

fn file_name(path: &Path) -> String {
    let name: String = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Audio".into())
        .chars()
        .filter(|c| !c.is_control())
        .take(elyvon_project::validate::limits::MAX_NAME_CHARS)
        .collect();
    if name.trim().is_empty() {
        "Audio".into()
    } else {
        name
    }
}

/// Decodes files off the UI thread, then adds them to the media pool and
/// places regions in a single undoable edit. Unreadable files are reported
/// individually; the others are still imported.
#[tauri::command]
pub async fn media_import(
    app: AppHandle,
    paths: Vec<String>,
    track_id: Option<TrackId>,
    start: Ticks,
    layout: ImportLayout,
) -> IpcResult<ImportResult> {
    if paths.is_empty() || paths.len() > 256 {
        return Err(IpcError::new(
            ErrorCode::InvalidEdit,
            "Choose between 1 and 256 files.",
        ));
    }
    let state = app.state::<AppState>();
    state.ensure_editable()?;
    let rate = state.audio.rate();
    let budget = state.audio.remaining_budget();
    log::info!("importing {} audio file(s)", paths.len());
    let decoded = tauri::async_runtime::spawn_blocking(move || {
        let mut budget = budget;
        paths
            .into_iter()
            .map(|p| {
                let path = PathBuf::from(&p);
                let result = MediaStore::load_file(&path, rate, budget);
                if let Ok(buf) = &result {
                    budget = budget.saturating_sub(buf.memory_bytes());
                }
                (path, result)
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(IpcError::from)?;

    let mut failures = Vec::new();
    let mut loaded: Vec<(MediaAsset, PathBuf, Arc<MediaBuffer>)> = Vec::new();
    for (path, result) in decoded {
        match result {
            Ok(buffer) => loaded.push((
                MediaAsset {
                    id: MediaId::new(),
                    name: file_name(&path),
                    path: path.display().to_string(),
                    info: Some(buffer.info.clone()),
                },
                path,
                buffer,
            )),
            Err(e) => {
                log::warn!("import of {} failed: {e}", path.display());
                failures.push(ImportFailure {
                    path: path.display().to_string(),
                    code: e.code().into(),
                    message: e.to_string(),
                });
            }
        }
    }
    if loaded.is_empty() {
        let message = failures
            .first()
            .map(|f| f.message.clone())
            .unwrap_or_else(|| "Nothing was imported.".into());
        return Err(IpcError::new(ErrorCode::Media, message));
    }

    let (view, outcome) = {
        let mut session = state.session();
        state.ensure_editable()?;
        let map = session
            .project()
            .transport
            .tempo_map()
            .map_err(|e| IpcError::new(ErrorCode::InvalidEdit, e.to_string()))?;
        let mut placements = Vec::with_capacity(loaded.len());
        let mut at = start;
        for (i, (asset, _, buffer)) in loaded.iter().enumerate() {
            let track = match (layout, track_id, i) {
                (ImportLayout::NewTracks, _, _) => PlacementTrack::New,
                (ImportLayout::Sequential, Some(id), 0) => PlacementTrack::Existing { id },
                (ImportLayout::Sequential, None, 0) => PlacementTrack::New,
                (ImportLayout::Sequential, _, _) => PlacementTrack::SameAsPrevious,
            };
            placements.push(AudioPlacement {
                media_id: asset.id,
                track,
                start: if layout == ImportLayout::NewTracks {
                    start
                } else {
                    at
                },
            });
            at = elyvon_project::timeline_edit::tick_after_frames(
                &map,
                at,
                buffer.info.frames.max(1),
                buffer.info.sample_rate,
            );
        }
        let assets = loaded.iter().map(|(a, _, _)| a.clone()).collect();
        let outcome = session
            .apply(&EditCommand::ImportAudio { assets, placements })
            .map_err(IpcError::from)?;
        (session.view(), outcome)
    };
    for (asset, path, buffer) in loaded {
        state.audio.insert_loaded(asset.id, &path, buffer);
    }
    state.audio.sync(&app);
    // Imported buffers bypass the background loader, which normally publishes
    // this event. Without it the UI stays at "Loading audio" until reloaded.
    let _ = app.emit(crate::audio::MEDIA_EVENT, state.audio.media_statuses(&app));
    Ok(ImportResult {
        view,
        outcome,
        failures,
    })
}

/// Points a pool entry at another file after decoding it successfully.
#[tauri::command]
pub async fn media_relink(app: AppHandle, id: MediaId, path: String) -> IpcResult<SessionView> {
    let state = app.state::<AppState>();
    state.ensure_editable()?;
    let rate = state.audio.rate();
    let budget = state.audio.remaining_budget();
    let p = PathBuf::from(&path);
    let load_path = p.clone();
    let buffer = tauri::async_runtime::spawn_blocking(move || {
        MediaStore::load_file(&load_path, rate, budget)
    })
    .await
    .map_err(IpcError::from)?
    .map_err(|e| IpcError::new(ErrorCode::Media, e.to_string()))?;
    let view = {
        let mut session = state.session();
        state.ensure_editable()?;
        session.apply(&EditCommand::RelinkMedia {
            id,
            path: p.display().to_string(),
            info: Some(buffer.info.clone()),
        })?;
        session.view()
    };
    state.audio.forget(id);
    state.audio.insert_loaded(id, &p, buffer);
    state.audio.sync(&app);
    let _ = app.emit(crate::audio::MEDIA_EVENT, state.audio.media_statuses(&app));
    Ok(view)
}

/// Waveform columns for the visible part of an audio region, as binary:
/// `u32 columns (LE)`, then per column `[minL, maxL, minR, maxR]` as `i16 LE`.
/// Errors with `notFound` while the file is not decoded (yet).
#[tauri::command]
pub async fn media_region_peaks(
    state: State<'_, AppState>,
    region_id: RegionId,
    from_tick: f64,
    to_tick: f64,
    columns: u32,
) -> IpcResult<Response> {
    let (region, map) = {
        let s = state.session();
        let p = s.project();
        let (ti, ri) = p
            .find_region(region_id)
            .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "The region no longer exists."))?;
        let map = p
            .transport
            .tempo_map()
            .map_err(|e| IpcError::internal(e.to_string()))?;
        (p.tracks[ti].regions[ri].clone(), map)
    };
    let RegionContent::Audio {
        media: Some(id), ..
    } = region.content
    else {
        return Err(IpcError::new(
            ErrorCode::NotFound,
            "The region has no audio file.",
        ));
    };
    let buffer = state
        .audio
        .buffer(id)
        .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "The audio file is not loaded."))?;
    if !(from_tick.is_finite() && to_tick.is_finite() && to_tick > from_tick) {
        return Err(IpcError::new(
            ErrorCode::InvalidEdit,
            "Invalid waveform range.",
        ));
    }
    let cols =
        waveform::region_columns(&map, &region, &buffer, from_tick, to_tick, columns as usize);
    let mut bytes = Vec::with_capacity(4 + cols.len() * 8);
    bytes.extend_from_slice(&(cols.len() as u32).to_le_bytes());
    let q = |v: f32| ((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes();
    for c in cols {
        for (lo, hi) in c {
            bytes.extend_from_slice(&q(lo));
            bytes.extend_from_slice(&q(hi));
        }
    }
    Ok(Response::new(bytes))
}

/// Renders the project offline at the engine rate and writes a 32-bit float
/// WAV plus `<file>.json` with the diagnostic report.
#[tauri::command]
pub async fn project_render(
    app: AppHandle,
    path: String,
    start_tick: Option<Ticks>,
    end_tick: Option<Ticks>,
    metronome: bool,
) -> IpcResult<RenderReport> {
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
                "Stop playback before exporting a diagnostic render.",
            ));
        }
        state
            .rendering
            .store(true, std::sync::atomic::Ordering::Release);
        session.project().clone()
    };
    struct RenderGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for RenderGuard {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::Release);
        }
    }
    let render_guard = RenderGuard(std::sync::Arc::clone(&state.rendering));
    let rate = state.audio.rate();
    let media: std::collections::HashMap<MediaId, Arc<MediaBuffer>> = project
        .media
        .iter()
        .filter_map(|a| state.audio.buffer(a.id).map(|b| (a.id, b)))
        .collect();
    let mut path = PathBuf::from(path);
    if path.extension().is_none() {
        path.set_extension("wav");
    }
    tauri::async_runtime::spawn_blocking(move || {
        let lookup = |id| media.get(&id).cloned();
        let _render_guard = render_guard;
        render::export_project(
            &path,
            &project,
            &lookup,
            RenderRequest {
                sample_rate: rate,
                start_tick: start_tick.unwrap_or(0),
                end_tick,
                block_frames: 1024,
                metronome,
            },
        )
        .map_err(|e| IpcError::new(ErrorCode::Io, e.to_string()))
    })
    .await
    .map_err(IpcError::from)?
}
