//! Process-wide application state managed by Tauri.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, PoisonError};

use elyvon_project::ProjectSession;

use crate::settings::Settings;
use crate::updater::UpdaterState;

/// Platform directories used by the application.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub recovery_dir: PathBuf,
    pub log_dir: PathBuf,
    pub home_dir: Option<PathBuf>,
}

#[allow(missing_debug_implementations)] // holds plugin types without Debug
pub struct AppState {
    /// The single open project. Locks are held only for in-memory work; disk
    /// IO happens outside the lock (see `commands::project`).
    pub session: Mutex<ProjectSession>,
    pub settings: Mutex<Settings>,
    pub updater: UpdaterState,
    pub paths: AppPaths,
    /// Set once the user confirmed quitting, so close/exit events pass through.
    pub exit_confirmed: AtomicBool,
}

impl AppState {
    pub fn new(paths: AppPaths, settings: Settings) -> Self {
        Self {
            session: Mutex::new(ProjectSession::new_empty()),
            settings: Mutex::new(settings),
            updater: UpdaterState::default(),
            paths,
            exit_confirmed: AtomicBool::new(false),
        }
    }

    pub fn session(&self) -> MutexGuard<'_, ProjectSession> {
        // A panic while holding the lock cannot leave the session half-edited
        // (edits are applied atomically), so recovering the guard is safe.
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn settings(&self) -> MutexGuard<'_, Settings> {
        self.settings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn is_dirty(&self) -> bool {
        self.session().is_dirty()
    }

    /// Deletes the crash-recovery snapshot (after save, discard or clean exit).
    pub fn clear_recovery(&self) {
        if let Err(e) = elyvon_project::recovery::clear(&self.paths.recovery_dir) {
            log::warn!("could not delete recovery snapshot: {e}");
        }
    }
}
