//! Process-wide application state managed by Tauri.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, PoisonError};

use elyvon_audio_device::manager::AudioManager;
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
    /// The audio device manager (its own control thread). `None` after exit.
    pub audio: Mutex<Option<Arc<AudioManager>>>,
    /// Set once the user confirmed quitting, so close/exit events pass through.
    pub exit_confirmed: AtomicBool,
}

impl AppState {
    pub fn new(paths: AppPaths, settings: Settings, audio: Option<AudioManager>) -> Self {
        Self {
            session: Mutex::new(ProjectSession::new_empty()),
            settings: Mutex::new(settings),
            updater: UpdaterState::default(),
            paths,
            audio: Mutex::new(audio.map(Arc::new)),
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

    /// The audio manager, cloned out of the lock so that slow device work
    /// (opening a driver can take seconds) never blocks other commands.
    pub fn audio(&self) -> Option<Arc<AudioManager>> {
        self.audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Stops all audio streams. The manager thread is joined when the last
    /// in-flight command releases its handle.
    pub fn shutdown_audio(&self) {
        let manager = self
            .audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(manager);
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
