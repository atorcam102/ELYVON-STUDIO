//! Project commands: the UI's only way to read and change the open project.

use std::path::PathBuf;

use elyvon_project::recovery::{self, RecoveryInfo};
use elyvon_project::{EditCommand, EditOutcome, ProjectSession, SessionView, format};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;

use crate::error::{ErrorCode, IpcError, IpcResult};
use crate::state::AppState;

pub(crate) fn app_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

fn ensure_can_discard(state: &AppState, discard_changes: bool) -> IpcResult<()> {
    if state.is_dirty() && !discard_changes {
        return Err(IpcError::unsaved());
    }
    Ok(())
}

/// Replaces the open session and performs the bookkeeping shared by
/// New/Open/Demo/Recover.
fn replace_session(
    app: &AppHandle,
    session: ProjectSession,
    recent: Option<PathBuf>,
) -> SessionView {
    let state = app.state::<AppState>();
    let view = {
        let mut s = state.session();
        *s = session;
        s.view()
    };
    state.clear_recovery();
    if let Some(path) = recent {
        remember_recent(app, path);
    }
    // Stop playback of the previous song and load the new one.
    state
        .audio
        .command(elyvon_audio::transport::TransportCommand::Stop);
    state
        .audio
        .command(elyvon_audio::transport::TransportCommand::Locate { tick: 0 });
    state.audio.sync(app);
    view
}

/// Adds a path to the recent-projects list and persists settings in the background.
fn remember_recent(app: &AppHandle, path: PathBuf) {
    let state = app.state::<AppState>();
    let settings = {
        let mut s = state.settings();
        s.add_recent(&path);
        s.clone()
    };
    let dir = state.paths.config_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = settings.save(&dir) {
            log::warn!("could not save settings: {e}");
        }
    });
}

#[tauri::command]
pub async fn project_get(state: State<'_, AppState>) -> IpcResult<SessionView> {
    Ok(state.session().view())
}

#[tauri::command]
pub async fn project_new(app: AppHandle, discard_changes: bool) -> IpcResult<SessionView> {
    ensure_can_discard(&app.state::<AppState>(), discard_changes)?;
    log::info!("new project");
    Ok(replace_session(&app, ProjectSession::new_empty(), None))
}

#[tauri::command]
pub async fn project_open_demo(app: AppHandle, discard_changes: bool) -> IpcResult<SessionView> {
    ensure_can_discard(&app.state::<AppState>(), discard_changes)?;
    log::info!("opening demo session");
    Ok(replace_session(&app, ProjectSession::demo(), None))
}

#[tauri::command]
pub async fn project_open(
    app: AppHandle,
    path: String,
    discard_changes: bool,
) -> IpcResult<SessionView> {
    ensure_can_discard(&app.state::<AppState>(), discard_changes)?;
    let path = PathBuf::from(path);
    log::info!("opening project {}", path.display());
    let load_path = path.clone();
    let session = tauri::async_runtime::spawn_blocking(move || ProjectSession::open(&load_path))
        .await
        .map_err(IpcError::from)?
        .map_err(|e| {
            log::warn!("failed to open {}: {e}", path.display());
            IpcError::from(e)
        })?;
    // The user may have edited while the file was loading.
    ensure_can_discard(&app.state::<AppState>(), discard_changes)?;
    Ok(replace_session(&app, session, Some(path)))
}

/// Saves the project. `path` is required for Save As (and for projects that
/// were never saved); otherwise the current file is overwritten atomically.
#[tauri::command]
pub async fn project_save(app: AppHandle, path: Option<String>) -> IpcResult<SessionView> {
    let state = app.state::<AppState>();
    let ticket = state
        .session()
        .begin_save(path.as_deref().map(std::path::Path::new))?;
    let version = app_version(&app);
    // Write outside the lock so the UI stays responsive during slow IO.
    let (ticket, result) = tauri::async_runtime::spawn_blocking(move || {
        let r = format::save(&ticket.path, &ticket.project, &version);
        (ticket, r)
    })
    .await
    .map_err(IpcError::from)?;
    if let Err(e) = result {
        log::error!("save failed: {e}");
        return Err(e.into());
    }
    let (saved, view) = {
        let mut s = state.session();
        let saved = s.complete_save(ticket);
        (saved, s.view())
    };
    log::info!("saved {}", saved.display());
    if !view.dirty {
        state.clear_recovery();
    }
    remember_recent(&app, saved);
    Ok(view)
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApplyResult {
    pub view: SessionView,
    pub outcome: EditOutcome,
}

#[tauri::command]
pub async fn project_apply(app: AppHandle, command: EditCommand) -> IpcResult<ApplyResult> {
    let state = app.state::<AppState>();
    let result = {
        let mut s = state.session();
        let outcome = s.apply(&command).map_err(|e| {
            log::debug!("edit rejected: {e}");
            IpcError::from(e)
        })?;
        ApplyResult {
            view: s.view(),
            outcome,
        }
    };
    // Publish the edit to the engine (outside the session lock).
    state.audio.sync(&app);
    Ok(result)
}

#[tauri::command]
pub async fn project_undo(app: AppHandle) -> IpcResult<SessionView> {
    let state = app.state::<AppState>();
    let view = {
        let mut s = state.session();
        s.undo();
        s.view()
    };
    state.audio.sync(&app);
    Ok(view)
}

#[tauri::command]
pub async fn project_redo(app: AppHandle) -> IpcResult<SessionView> {
    let state = app.state::<AppState>();
    let view = {
        let mut s = state.session();
        s.redo();
        s.view()
    };
    state.audio.sync(&app);
    Ok(view)
}

/// Writes a crash-recovery snapshot if there are unsaved changes.
#[tauri::command]
pub async fn recovery_snapshot(app: AppHandle) -> IpcResult<()> {
    let state = app.state::<AppState>();
    let (project, path) = {
        let s = state.session();
        if !s.is_dirty() {
            return Ok(());
        }
        (s.project().clone(), s.path().map(PathBuf::from))
    };
    let dir = state.paths.recovery_dir.clone();
    let version = app_version(&app);
    tauri::async_runtime::spawn_blocking(move || {
        recovery::write(&dir, &project, path.as_deref(), &version)
    })
    .await
    .map_err(IpcError::from)?
    .map_err(|e| {
        log::warn!("could not write recovery snapshot: {e}");
        IpcError::new(ErrorCode::Io, e.to_string())
    })
}

/// Reports a snapshot left behind by a previous run that did not exit cleanly.
#[tauri::command]
pub async fn recovery_check(app: AppHandle) -> IpcResult<Option<RecoveryInfo>> {
    let dir = app.state::<AppState>().paths.recovery_dir.clone();
    let found = tauri::async_runtime::spawn_blocking(move || recovery::read(&dir))
        .await
        .map_err(IpcError::from)?;
    match found {
        Ok(found) => Ok(found.map(|(info, _)| info)),
        Err(e) => {
            log::warn!("discarding unreadable recovery snapshot: {e}");
            app.state::<AppState>().clear_recovery();
            Ok(None)
        }
    }
}

#[tauri::command]
pub async fn recovery_restore(app: AppHandle) -> IpcResult<SessionView> {
    let dir = app.state::<AppState>().paths.recovery_dir.clone();
    let found = tauri::async_runtime::spawn_blocking(move || recovery::read(&dir))
        .await
        .map_err(IpcError::from)??;
    let (info, project) =
        found.ok_or_else(|| IpcError::new(ErrorCode::NotFound, "There is nothing to recover."))?;
    log::info!("restoring recovery snapshot of {:?}", info.project_name);
    let session = ProjectSession::recovered(project, info.original_path.map(PathBuf::from));
    let state = app.state::<AppState>();
    let view = {
        let mut s = state.session();
        *s = session;
        s.view()
    };
    state.audio.sync(&app);
    // Keep the snapshot until the user saves: a second crash must not lose it.
    Ok(view)
}

#[tauri::command]
pub async fn recovery_discard(state: State<'_, AppState>) -> IpcResult<()> {
    state.clear_recovery();
    Ok(())
}
