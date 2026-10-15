//! Application-level commands: info, settings, quitting, updates.

use std::sync::atomic::Ordering;

use elyvon_core::jobs::{ContractVersions, IPC_CONTRACT_VERSION, JOB_CONTRACT_VERSION};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;

use crate::error::{ErrorCode, IpcError, IpcResult};
use crate::settings::Settings;
use crate::state::AppState;
use crate::updater::{self, DownloadEvent, InstallResult, UpdateCheckResult, UpdaterEnvironment};

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub rust_target: Option<String>,
    pub debug_build: bool,
    pub log_dir: String,
    pub config_dir: String,
    pub updater: UpdaterEnvironment,
}

#[tauri::command]
pub async fn app_info(app: AppHandle, state: State<'_, AppState>) -> IpcResult<AppInfo> {
    let env = UpdaterEnvironment::detect(&app);
    Ok(AppInfo {
        name: app.package_info().name.clone(),
        version: app.package_info().version.to_string(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        rust_target: env.platform.map(|p| p.rust_target().to_owned()),
        debug_build: cfg!(debug_assertions),
        log_dir: state.paths.log_dir.display().to_string(),
        config_dir: state.paths.config_dir.display().to_string(),
        updater: env,
    })
}

/// Read-only version negotiation. Does not imply a graph executor or job worker.
#[tauri::command]
pub async fn app_contracts() -> IpcResult<ContractVersions> {
    Ok(ContractVersions {
        ipc: IPC_CONTRACT_VERSION,
        project: elyvon_project::format::CURRENT_SCHEMA_VERSION,
        audio: elyvon_audio::contracts::AUDIO_CONTRACT_VERSION,
        jobs: JOB_CONTRACT_VERSION,
    })
}

#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> IpcResult<Settings> {
    Ok(state.settings().clone())
}

#[tauri::command]
pub async fn settings_set(app: AppHandle, settings: Settings) -> IpcResult<Settings> {
    let state = app.state::<AppState>();
    let settings = {
        let mut current = state.settings();
        // Audio preferences change only through `audio_configure`, so a UI
        // holding an older copy can never overwrite the device choice.
        let settings = Settings {
            audio: current.audio.clone(),
            ..settings
        }
        .sanitized();
        *current = settings.clone();
        settings
    };
    let dir = state.paths.config_dir.clone();
    let to_save = settings.clone();
    tauri::async_runtime::spawn_blocking(move || to_save.save(&dir))
        .await
        .map_err(IpcError::from)?
        .map_err(|e| {
            log::warn!("could not save settings: {e}");
            IpcError::new(ErrorCode::Io, format!("Could not save preferences: {e}"))
        })?;
    Ok(settings)
}

/// Quits after the UI resolved unsaved changes. Refuses to drop unsaved work
/// unless `discard_changes` is set explicitly.
#[tauri::command]
pub async fn app_quit(app: AppHandle, discard_changes: bool) -> IpcResult<()> {
    let state = app.state::<AppState>();
    if state.is_dirty() && !discard_changes {
        return Err(IpcError::unsaved());
    }
    log::info!("quitting");
    state.exit_confirmed.store(true, Ordering::SeqCst);
    state.clear_recovery();
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    state: State<'_, AppState>,
) -> IpcResult<UpdateCheckResult> {
    updater::check(&app, &state.updater)
        .await
        .inspect_err(|e| log::warn!("update check failed: {e}"))
}

#[tauri::command]
pub async fn update_download(
    state: State<'_, AppState>,
    on_event: Channel<DownloadEvent>,
) -> IpcResult<()> {
    updater::download(&state.updater, on_event)
        .await
        .inspect_err(|e| log::warn!("update download failed: {e}"))
}

#[tauri::command]
pub async fn update_cancel(state: State<'_, AppState>) -> IpcResult<bool> {
    Ok(updater::cancel(&state.updater))
}

/// Installs the downloaded update. Refused while the project has unsaved
/// changes: on Windows the installer terminates the application.
#[tauri::command]
pub async fn update_install(state: State<'_, AppState>) -> IpcResult<InstallResult> {
    if state.is_dirty() {
        return Err(IpcError::unsaved());
    }
    // On Windows `install` exits the process; make that a clean exit.
    state.exit_confirmed.store(true, Ordering::SeqCst);
    state.clear_recovery();
    let result = updater::install(&state.updater);
    if result.is_err() {
        state.exit_confirmed.store(false, Ordering::SeqCst);
    }
    result.inspect_err(|e| log::error!("update install failed: {e}"))
}

/// Restarts into the freshly installed version. Never with unsaved changes.
#[tauri::command]
pub async fn app_restart(app: AppHandle) -> IpcResult<()> {
    let state = app.state::<AppState>();
    if state.is_dirty() {
        return Err(IpcError::unsaved());
    }
    log::info!("restarting");
    state.exit_confirmed.store(true, Ordering::SeqCst);
    state.clear_recovery();
    app.restart()
}
