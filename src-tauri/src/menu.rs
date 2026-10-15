//! Native application menu.
//!
//! macOS gets the conventional application menu (About, Settings…, Check for
//! Updates…, Services, Hide, Quit); Windows and Linux put Settings and Exit in
//! File and About/Updates in Help. `CmdOrCtrl` maps to ⌘ on macOS and Ctrl
//! elsewhere. Every custom item is forwarded to the UI as a `menu` event whose
//! payload is the item id, so the UI owns dialogs such as unsaved-changes
//! prompts.
//!
//! Single-key shortcuts (I, B, E, X, Space, Delete…) are deliberately *not*
//! menu accelerators: on Windows and Linux they would swallow typing in text
//! fields. The UI handles them only when no text field has focus.

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Runtime};

pub const EVENT: &str = "menu";

fn item<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    text: &str,
    accel: Option<&str>,
) -> tauri::Result<tauri::menu::MenuItem<R>> {
    let mut b = MenuItemBuilder::with_id(id, text);
    if let Some(a) = accel {
        b = b.accelerator(a);
    }
    b.build(app)
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let macos = cfg!(target_os = "macos");

    let mut file = SubmenuBuilder::new(app, "File")
        .item(&item(app, "file.new", "New Project", Some("CmdOrCtrl+N"))?)
        .item(&item(app, "file.open", "Open…", Some("CmdOrCtrl+O"))?)
        .item(&item(app, "file.openDemo", "Open Demo Session", None)?)
        .separator()
        .item(&item(app, "file.save", "Save", Some("CmdOrCtrl+S"))?)
        .item(&item(
            app,
            "file.saveAs",
            "Save As…",
            Some("CmdOrCtrl+Shift+S"),
        )?);
    if !macos {
        file = file
            .separator()
            .item(&item(app, "app.settings", "Settings…", Some("Ctrl+Comma"))?)
            .separator()
            .item(&item(app, "app.quit", "Exit", Some("Ctrl+Q"))?);
    }

    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&item(app, "edit.undo", "Undo", Some("CmdOrCtrl+Z"))?)
        .item(&item(app, "edit.redo", "Redo", Some("CmdOrCtrl+Shift+Z"))?)
        .separator()
        // Native clipboard items keep text fields working (required on macOS).
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;

    let track = SubmenuBuilder::new(app, "Track")
        .item(&item(
            app,
            "track.newAudio",
            "New Audio Track",
            Some("CmdOrCtrl+Alt+A"),
        )?)
        .item(&item(
            app,
            "track.newInstrument",
            "New Instrument Track",
            Some("CmdOrCtrl+Alt+S"),
        )?)
        .item(&item(
            app,
            "track.newMidi",
            "New MIDI Track",
            Some("CmdOrCtrl+Alt+X"),
        )?)
        .separator()
        .item(&item(
            app,
            "track.duplicate",
            "Duplicate Selected Tracks",
            Some("CmdOrCtrl+D"),
        )?)
        .item(&item(app, "track.delete", "Delete Selected Tracks", None)?)
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&item(app, "view.inspector", "Toggle Inspector", None)?)
        .item(&item(app, "view.browser", "Toggle Browser", None)?)
        .item(&item(app, "view.bottom", "Toggle Editors", None)?)
        .item(&item(app, "view.mixer", "Show Mixer", None)?)
        .separator()
        .item(&item(
            app,
            "view.zoomIn",
            "Zoom In",
            Some("CmdOrCtrl+Equal"),
        )?)
        .item(&item(
            app,
            "view.zoomOut",
            "Zoom Out",
            Some("CmdOrCtrl+Minus"),
        )?)
        .build()?;

    let mut menu = MenuBuilder::new(app);
    if macos {
        let app_menu = SubmenuBuilder::new(app, "Elyvon Studio")
            .item(&item(app, "app.about", "About Elyvon Studio", None)?)
            .separator()
            .item(&item(app, "app.settings", "Settings…", Some("Cmd+Comma"))?)
            .item(&item(app, "app.checkUpdates", "Check for Updates…", None)?)
            .separator()
            .item(&PredefinedMenuItem::services(app, None)?)
            .separator()
            .item(&PredefinedMenuItem::hide(app, None)?)
            .item(&PredefinedMenuItem::hide_others(app, None)?)
            .item(&PredefinedMenuItem::show_all(app, None)?)
            .separator()
            .item(&item(app, "app.quit", "Quit Elyvon Studio", Some("Cmd+Q"))?)
            .build()?;
        menu = menu.item(&app_menu);
    }
    menu = menu
        .item(&file.build()?)
        .item(&edit)
        .item(&track)
        .item(&view);
    if macos {
        let window = SubmenuBuilder::new(app, "Window")
            .item(&PredefinedMenuItem::minimize(app, None)?)
            .item(&PredefinedMenuItem::maximize(app, None)?)
            .build()?;
        menu = menu.item(&window);
    } else {
        let help = SubmenuBuilder::new(app, "Help")
            .item(&item(app, "app.checkUpdates", "Check for Updates…", None)?)
            .separator()
            .item(&item(app, "app.about", "About Elyvon Studio", None)?)
            .build()?;
        menu = menu.item(&help);
    }
    menu.build()
}
