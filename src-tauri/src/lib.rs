//! Elyvon Studio desktop shell.
//!
//! This crate wires the native core (`elyvon-*` crates) to the webview UI via
//! Tauri commands and events. It owns process concerns only: windows, menus,
//! logging, preferences, crash recovery and updates. Musical logic lives in
//! the core crates; the webview is purely a presentation layer.

pub mod commands;
pub mod error;
pub mod menu;
pub mod settings;
pub mod state;
pub mod updater;

use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

use crate::settings::Settings;
use crate::state::{AppPaths, AppState};

/// Emitted when the user tries to close the window or quit with unsaved changes.
pub const CLOSE_REQUESTED_EVENT: &str = "app://close-requested";

fn log_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_log::Builder::new()
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::LogDir {
                file_name: Some("elyvon-studio".into()),
            }),
        ])
        .level(if cfg!(debug_assertions) {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        // Keep third-party crates quiet unless something is wrong.
        .level_for("tao", log::LevelFilter::Warn)
        .level_for("wry", log::LevelFilter::Warn)
        .level_for("reqwest", log::LevelFilter::Warn)
        .level_for("hyper_util", log::LevelFilter::Warn)
        .max_file_size(5 * 1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepSome(3))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .build()
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default(info);
    }));
}

/// Intercepts window close / app exit while the project has unsaved changes
/// and hands the decision to the UI (which calls `app_quit`).
fn request_close(app: &tauri::AppHandle) -> bool {
    let state = app.state::<AppState>();
    if state.exit_confirmed.load(Ordering::SeqCst) {
        return false;
    }
    if state.is_dirty() {
        let _ = app.emit(CLOSE_REQUESTED_EVENT, ());
        true
    } else {
        state.clear_recovery();
        false
    }
}

pub fn run() {
    install_panic_hook();
    let app = tauri::Builder::default()
        .plugin(log_plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let path = app.path();
            let paths = AppPaths {
                config_dir: path.app_config_dir()?,
                recovery_dir: path.app_data_dir()?.join("recovery"),
                log_dir: path.app_log_dir()?,
                home_dir: path.home_dir().ok(),
            };
            log::info!(
                "Elyvon Studio {} starting ({} {})",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            let settings = Settings::load(&paths.config_dir);
            app.manage(AppState::new(paths, settings));
            app.set_menu(menu::build(app.handle())?)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().0.as_str();
            if id == "app.quit" {
                if !request_close(app) {
                    app.state::<AppState>()
                        .exit_confirmed
                        .store(true, Ordering::SeqCst);
                    app.exit(0);
                }
                return;
            }
            if let Err(e) = app.emit(menu::EVENT, id) {
                log::error!("could not forward menu event {id}: {e}");
            }
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && request_close(window.app_handle())
            {
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::project::project_get,
            commands::project::project_new,
            commands::project::project_open,
            commands::project::project_open_demo,
            commands::project::project_save,
            commands::project::project_apply,
            commands::project::project_undo,
            commands::project::project_redo,
            commands::project::recovery_snapshot,
            commands::project::recovery_check,
            commands::project::recovery_restore,
            commands::project::recovery_discard,
            commands::browser::browser_list,
            commands::browser::browser_places,
            commands::system::app_info,
            commands::system::app_contracts,
            commands::system::audio_status,
            commands::system::settings_get,
            commands::system::settings_set,
            commands::system::app_quit,
            commands::system::app_restart,
            commands::system::update_check,
            commands::system::update_download,
            commands::system::update_cancel,
            commands::system::update_install,
        ])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            log::error!("failed to start: {e}");
            eprintln!("Elyvon Studio failed to start: {e}");
            std::process::exit(1);
        }
    };

    app.run(|app, event| {
        // Dock "Quit", logout, Cmd+Q from the system: same unsaved-changes guard.
        if let RunEvent::ExitRequested {
            code: None, api, ..
        } = event
            && request_close(app)
        {
            api.prevent_exit();
        }
    });
}
