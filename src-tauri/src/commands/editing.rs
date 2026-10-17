//! Automation capture and non-destructive editing commands that need the
//! engine or the disk (Phases 10–11). Pure project edits go through
//! `project_apply`; these measure or render audio first and then apply one
//! undoable edit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use elyvon_audio::analysis;
use elyvon_audio::media::repair::{self, RepairReport};
use elyvon_audio::media::{MediaBuffer, MediaStatus, MediaStore};
use elyvon_core::Ticks;
use elyvon_project::automation::AutomationParam;
use elyvon_project::automation_record::TouchPhase;
use elyvon_project::mixer::ChannelRef;
use elyvon_project::region_edit::RegionGain;
use elyvon_project::{
    EditCommand, MediaAsset, MediaId, Project, RegionContent, RegionId, SessionView, TrackId,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::commands::project::ApplyResult;
use crate::error::{ErrorCode, IpcError, IpcResult};
use crate::state::AppState;

fn busy(msg: &str) -> IpcError {
    IpcError::new(ErrorCode::Busy, msg)
}

fn invalid(msg: impl Into<String>) -> IpcError {
    IpcError::new(ErrorCode::InvalidEdit, msg)
}

/// A live control gesture (fader, pan, mute, send, insert parameter).
/// Returns whether it was written as automation; when `false` the UI
/// applies the value as an ordinary edit.
#[tauri::command]
pub async fn automation_touch(
    app: AppHandle,
    channel: ChannelRef,
    param: AutomationParam,
    value: f64,
    phase: TouchPhase,
) -> IpcResult<bool> {
    Ok(app
        .state::<AppState>()
        .automation
        .touch(&app, channel, param, value, phase))
}

fn not_playing(state: &AppState) -> IpcResult<()> {
    if state.audio.snapshot().is_some_and(|s| {
        matches!(
            s.state,
            elyvon_audio::transport::PlayState::Playing
                | elyvon_audio::transport::PlayState::CountIn
        )
    }) {
        return Err(busy("Stop playback first."));
    }
    Ok(())
}

fn apply(app: &AppHandle, cmd: &EditCommand) -> IpcResult<ApplyResult> {
    let state = app.state::<AppState>();
    let result = {
        let mut s = state.session();
        state.ensure_editable()?;
        let outcome = s.apply(cmd)?;
        ApplyResult {
            view: s.view(),
            outcome,
        }
    };
    state.audio.sync(app);
    Ok(result)
}

fn region_and_buffer(
    state: &AppState,
    id: RegionId,
) -> IpcResult<(
    elyvon_project::Region,
    elyvon_core::TempoMap,
    Arc<MediaBuffer>,
)> {
    let s = state.session();
    let p = s.project();
    let (ti, ri) = p
        .find_region(id)
        .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "The region no longer exists."))?;
    let region = p.tracks[ti].regions[ri].clone();
    let map = p
        .transport
        .tempo_map()
        .map_err(|e| IpcError::internal(e.to_string()))?;
    let RegionContent::Audio {
        media: Some(media), ..
    } = region.content
    else {
        return Err(invalid("Choose an audio region with a file."));
    };
    let buffer = state.audio.buffer(media).ok_or_else(|| {
        IpcError::new(
            ErrorCode::Media,
            "The region's audio file is not loaded (missing or still loading).",
        )
    })?;
    Ok((region, map, buffer))
}

/// Sets each region's gain so its peak reaches `target_db` (non-destructive).
#[tauri::command]
pub async fn region_normalize(
    app: AppHandle,
    ids: Vec<RegionId>,
    target_db: f64,
) -> IpcResult<ApplyResult> {
    if ids.is_empty() || ids.len() > 4096 || !(-60.0..=0.0).contains(&target_db) {
        return Err(invalid(
            "Choose 1–4096 regions and a target between −60 and 0 dBFS.",
        ));
    }
    let state = app.state::<AppState>();
    state.ensure_editable()?;
    let mut jobs = Vec::with_capacity(ids.len());
    for id in &ids {
        jobs.push((*id, region_and_buffer(&state, *id)?));
    }
    let gains = tauri::async_runtime::spawn_blocking(move || {
        jobs.into_iter()
            .filter_map(|(id, (region, map, buffer))| {
                let peak = analysis::region_peak(&map, &region, &buffer)?;
                Some(RegionGain {
                    id,
                    gain_db: analysis::normalize_gain(peak, target_db)?,
                })
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(IpcError::from)?;
    if gains.is_empty() {
        return Err(invalid("The selected regions are silent."));
    }
    apply(&app, &EditCommand::NormalizeRegions { gains })
}

/// Onsets in a region (absolute ticks) for markers or slices.
#[tauri::command]
pub async fn region_transients(
    app: AppHandle,
    id: RegionId,
    sensitivity: f64,
) -> IpcResult<Vec<Ticks>> {
    let state = app.state::<AppState>();
    let (region, map, buffer) = region_and_buffer(&state, id)?;
    tauri::async_runtime::spawn_blocking(move || {
        analysis::detect_transients(&map, &region, &buffer, sensitivity.clamp(0.0, 1.0))
    })
    .await
    .map_err(IpcError::from)
}

/// Folder for files Elyvon renders for this project: next to a saved
/// project, otherwise in the application data folder.
fn render_dir(state: &AppState, project: &Project, path: Option<&Path>, kind: &str) -> PathBuf {
    match path.and_then(Path::parent) {
        Some(dir) => dir.join(kind),
        None => state
            .paths
            .recovery_dir
            .parent()
            .unwrap_or(&state.paths.recovery_dir)
            .join("renders")
            .join(project.id.to_string())
            .join(kind),
    }
}

fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let s = s.trim().to_owned();
    if s.is_empty() { "Track".into() } else { s }
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("{secs}")
}

struct RenderGuard(Arc<std::sync::atomic::AtomicBool>);
impl Drop for RenderGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Renders a track (or some of its regions) to a new WAV and loads it.
async fn render_to_file(
    app: &AppHandle,
    track: TrackId,
    regions: Option<Vec<RegionId>>,
    include_fader: bool,
    kind: &str,
) -> IpcResult<(MediaAsset, Ticks, Arc<MediaBuffer>, PathBuf)> {
    let state = app.state::<AppState>();
    not_playing(&state)?;
    let (project, path) = {
        let s = state.session();
        state.ensure_editable()?;
        (s.project().clone(), s.path().map(Path::to_path_buf))
    };
    let name = project
        .track(track)
        .map(|t| t.name.clone())
        .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "The track no longer exists."))?;
    let media: HashMap<MediaId, Arc<MediaBuffer>> = state.audio.buffers(&project);
    let dir = render_dir(&state, &project, path.as_deref(), kind);
    let file = dir.join(format!("{} {}.wav", safe_name(&name), stamp()));
    let rate = state.audio.rate();
    state
        .rendering
        .store(true, std::sync::atomic::Ordering::Release);
    let guard = RenderGuard(Arc::clone(&state.rendering));
    let target = file.clone();
    let rendered = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let lookup = |id| media.get(&id).cloned();
        let r = analysis::render_track(
            &project,
            &lookup,
            track,
            regions.as_deref(),
            include_fader,
            2.0,
            rate,
        )?;
        if r.missing_media > 0 {
            return Err(format!(
                "{} region(s) have no audio file loaded; locate the missing files before rendering",
                r.missing_media
            ));
        }
        analysis::write_render(&target, &r)
            .map_err(|e| format!("could not write {}: {e}", target.display()))?;
        Ok::<_, String>(r)
    })
    .await
    .map_err(IpcError::from)?
    .map_err(|e| IpcError::new(ErrorCode::Media, e))?;
    // Verify what was written by decoding it back.
    let budget = state.audio.remaining_budget();
    let load = file.clone();
    let buffer =
        tauri::async_runtime::spawn_blocking(move || MediaStore::load_file(&load, rate, budget))
            .await
            .map_err(IpcError::from)?
            .map_err(|e| IpcError::new(ErrorCode::Media, e.to_string()))?;
    if buffer.frames() != rendered.frames() {
        return Err(IpcError::new(
            ErrorCode::Media,
            "The rendered file could not be verified.",
        ));
    }
    let asset = MediaAsset {
        id: MediaId::new(),
        name: file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Render.wav".into()),
        path: file.display().to_string(),
        info: Some(buffer.info.clone()),
    };
    Ok((asset, rendered.start, buffer, file))
}

/// Renders a track through its inserts (pre-fader) and plays that file
/// instead of processing the track live.
#[tauri::command]
pub async fn track_freeze(app: AppHandle, id: TrackId) -> IpcResult<ApplyResult> {
    let (asset, start, buffer, file) = render_to_file(&app, id, None, false, "Freeze").await?;
    let state = app.state::<AppState>();
    let media_id = asset.id;
    let result = apply(
        &app,
        &EditCommand::FreezeTrack {
            id,
            media: asset,
            start,
        },
    )?;
    state.audio.insert_loaded(media_id, &file, buffer);
    state.audio.sync(&app);
    let _ = app.emit(crate::audio::MEDIA_EVENT, state.audio.media_statuses(&app));
    log::info!("froze track {id} into {}", file.display());
    Ok(result)
}

/// Renders regions of a track to a new file on a new track below it.
#[tauri::command]
pub async fn track_bounce(
    app: AppHandle,
    track_id: TrackId,
    region_ids: Option<Vec<RegionId>>,
    include_fader: bool,
    mute_sources: bool,
) -> IpcResult<ApplyResult> {
    let sources: Vec<RegionId> = match &region_ids {
        Some(ids) if !ids.is_empty() => ids.clone(),
        _ => {
            let s = app.state::<AppState>();
            let s = s.session();
            s.project()
                .track(track_id)
                .map(|t| {
                    t.regions
                        .iter()
                        .filter(|r| !r.muted)
                        .map(|r| r.id)
                        .collect()
                })
                .unwrap_or_default()
        }
    };
    let (asset, start, buffer, file) = render_to_file(
        &app,
        track_id,
        region_ids.filter(|v| !v.is_empty()),
        include_fader,
        "Bounces",
    )
    .await?;
    let state = app.state::<AppState>();
    let media_id = asset.id;
    let result = apply(
        &app,
        &EditCommand::BounceInPlace {
            asset,
            start,
            source_track: track_id,
            regions: sources,
            mute_sources,
        },
    )?;
    state.audio.insert_loaded(media_id, &file, buffer);
    state.audio.sync(&app);
    let _ = app.emit(crate::audio::MEDIA_EVENT, state.audio.media_statuses(&app));
    Ok(result)
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairResult {
    pub view: SessionView,
    pub report: RepairReport,
}

fn project_dir(state: &AppState) -> Option<PathBuf> {
    state
        .session()
        .path()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// Searches `directory` for missing files and relinks the verified matches
/// in one undoable edit.
#[tauri::command]
pub async fn media_repair(app: AppHandle, directory: String) -> IpcResult<RepairResult> {
    let state = app.state::<AppState>();
    state.ensure_editable()?;
    let project = state.session().project().clone();
    let dir = project_dir(&state);
    let root = PathBuf::from(directory);
    if !root.is_dir() {
        return Err(IpcError::new(
            ErrorCode::NotFound,
            "Choose a folder to search.",
        ));
    }
    let report = tauri::async_runtime::spawn_blocking(move || {
        repair::find_missing(&project, dir.as_deref(), &root, 200_000)
    })
    .await
    .map_err(IpcError::from)?;
    let view = if report.relinked.is_empty() {
        state.session().view()
    } else {
        let ids: Vec<MediaId> = report.relinked.iter().map(|r| r.id).collect();
        let r = apply(
            &app,
            &EditCommand::RelinkMediaFiles {
                items: report.relinked.clone(),
            },
        )?;
        for id in ids {
            state.audio.forget(id);
        }
        state.audio.sync(&app);
        r.view
    };
    Ok(RepairResult { view, report })
}

/// Copies every file the project uses into its `Media` folder and points
/// the project at the copies (the originals are left untouched).
#[tauri::command]
pub async fn media_collect(app: AppHandle) -> IpcResult<SessionView> {
    let state = app.state::<AppState>();
    state.ensure_editable()?;
    let dir = project_dir(&state).ok_or_else(|| {
        IpcError::new(
            ErrorCode::NeedsSaveAs,
            "Save the project first: media are collected next to the project file.",
        )
    })?;
    let project = state.session().project().clone();
    let relinks = tauri::async_runtime::spawn_blocking(move || repair::collect(&project, &dir))
        .await
        .map_err(IpcError::from)?
        .map_err(|e| IpcError::new(ErrorCode::Io, e))?;
    if relinks.is_empty() {
        return Ok(state.session().view());
    }
    let view = apply(&app, &EditCommand::RelinkMediaFiles { items: relinks })?.view;
    Ok(view)
}

/// Files in the project's Media/Bounces/Freeze folders it no longer uses.
#[tauri::command]
pub async fn media_unused(app: AppHandle) -> IpcResult<Vec<String>> {
    let state = app.state::<AppState>();
    let Some(dir) = project_dir(&state) else {
        return Ok(Vec::new());
    };
    let project = state.session().project().clone();
    Ok(repair::unused_files(&project, &dir)
        .into_iter()
        .map(|p| p.display().to_string())
        .collect())
}

/// Moves the unused files into `.elyvon-backup/<time>/` in the project
/// folder. Destructive for the folder layout, so it needs `confirm`.
#[tauri::command]
pub async fn media_backup_unused(app: AppHandle, confirm: bool) -> IpcResult<String> {
    if !confirm {
        return Err(IpcError::new(
            ErrorCode::Cancelled,
            "Confirm moving the unused files to a backup folder.",
        ));
    }
    let state = app.state::<AppState>();
    state.ensure_editable()?;
    let dir = project_dir(&state)
        .ok_or_else(|| IpcError::new(ErrorCode::NeedsSaveAs, "Save the project first."))?;
    let project = state.session().project().clone();
    let files = repair::unused_files(&project, &dir);
    if files.is_empty() {
        return Err(IpcError::new(
            ErrorCode::NotFound,
            "There are no unused files.",
        ));
    }
    let backup = repair::move_to_backup(&files, &dir, &stamp())
        .map_err(|e| IpcError::new(ErrorCode::Io, e))?;
    log::info!(
        "moved {} unused file(s) to {}",
        files.len(),
        backup.display()
    );
    Ok(backup.display().to_string())
}

/// Forgets failed media (missing, unreadable) so they are loaded again.
#[tauri::command]
pub async fn media_rescan(app: AppHandle) -> IpcResult<Vec<MediaStatus>> {
    let state = app.state::<AppState>();
    let failed: Vec<MediaId> = state
        .audio
        .media_statuses(&app)
        .into_iter()
        .filter(|s| matches!(s.state, elyvon_audio::media::MediaState::Failed { .. }))
        .map(|s| s.id)
        .collect();
    for id in failed {
        state.audio.forget(id);
    }
    state.audio.sync(&app);
    Ok(state.audio.media_statuses(&app))
}
