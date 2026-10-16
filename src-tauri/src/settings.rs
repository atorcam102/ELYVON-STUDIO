//! Application preferences, persisted as JSON in the platform config directory.
//!
//! Loading never fails: missing fields take defaults and a corrupt file is
//! moved aside (`settings.json.corrupt`) and replaced by defaults, so a bad
//! preferences file can never prevent the application from starting.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const SETTINGS_FILE: &str = "settings.json";
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;
pub const MAX_RECENT: usize = 10;
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    pub schema_version: u32,
    pub updates: UpdateSettings,
    pub recent_projects: Vec<String>,
    pub layout: Layout,
    pub browser_dir: Option<String>,
    pub audio: AudioSettings,
}

/// Audio output preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AudioSettings {
    /// Use an audio device. When off (or when no device works) the engine
    /// runs on its internal clock without sound.
    pub use_device: bool,
    /// Device id from `audio_devices`; `None` = system default.
    pub output_device: Option<String>,
    /// Requested buffer size in frames; 0 = device default.
    pub buffer_frames: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            use_device: true,
            output_device: None,
            buffer_frames: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct UpdateSettings {
    /// Check once, in the background, a few seconds after start-up.
    pub check_on_startup: bool,
}

/// Panel visibility and sizes (logical pixels).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Layout {
    pub inspector_visible: bool,
    pub browser_visible: bool,
    pub bottom_visible: bool,
    pub inspector_width: f64,
    pub browser_width: f64,
    pub bottom_height: f64,
    pub track_header_width: f64,
    pub bottom_tab: String,
    pub browser_tab: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            updates: UpdateSettings::default(),
            recent_projects: Vec::new(),
            layout: Layout::default(),
            browser_dir: None,
            audio: AudioSettings::default(),
        }
    }
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_on_startup: true,
        }
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            inspector_visible: true,
            browser_visible: true,
            bottom_visible: true,
            inspector_width: 240.0,
            browser_width: 260.0,
            bottom_height: 340.0,
            track_header_width: 220.0,
            bottom_tab: "mixer".into(),
            browser_tab: "files".into(),
        }
    }
}

impl Settings {
    /// Clamps values coming from disk or from the UI into sane ranges.
    pub fn sanitized(mut self) -> Self {
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        let l = &mut self.layout;
        let clamp = |v: f64, min: f64, max: f64, default: f64| {
            if v.is_finite() {
                v.clamp(min, max)
            } else {
                default
            }
        };
        let d = Layout::default();
        l.inspector_width = clamp(l.inspector_width, 180.0, 480.0, d.inspector_width);
        l.browser_width = clamp(l.browser_width, 180.0, 560.0, d.browser_width);
        l.bottom_height = clamp(l.bottom_height, 140.0, 720.0, d.bottom_height);
        l.track_header_width = clamp(l.track_header_width, 160.0, 420.0, d.track_header_width);
        if !["mixer", "pianoRoll", "audioEditor"].contains(&l.bottom_tab.as_str()) {
            l.bottom_tab = d.bottom_tab;
        }
        if !["files", "loops", "instruments", "plugins"].contains(&l.browser_tab.as_str()) {
            l.browser_tab = d.browser_tab;
        }
        let a = &mut self.audio;
        if a.buffer_frames != 0 {
            a.buffer_frames = a.buffer_frames.clamp(32, 8192);
        }
        if a.output_device.as_deref().is_some_and(|d| d.is_empty() || d.len() > 512) {
            a.output_device = None;
        }
        self.recent_projects.retain(|p| !p.is_empty());
        self.recent_projects.dedup();
        self.recent_projects.truncate(MAX_RECENT);
        self
    }

    pub fn add_recent(&mut self, path: &Path) {
        let p = path.display().to_string();
        self.recent_projects.retain(|r| r != &p);
        self.recent_projects.insert(0, p);
        self.recent_projects.truncate(MAX_RECENT);
    }

    /// Loads settings from `dir`, falling back to defaults on any problem.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(SETTINGS_FILE);
        let bytes = match elyvon_core::fs::read_limited(&path, MAX_SETTINGS_BYTES) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                log::warn!("could not read {}: {e}; using defaults", path.display());
                return Self::default();
            }
        };
        match serde_json::from_slice::<Settings>(&bytes) {
            Ok(s) => s.sanitized(),
            Err(e) => {
                let aside = corrupt_path(&path);
                log::warn!(
                    "settings file is corrupt ({e}); moving it to {}",
                    aside.display()
                );
                if let Err(e) = std::fs::rename(&path, &aside) {
                    log::warn!("could not move corrupt settings aside: {e}");
                }
                Self::default()
            }
        }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        bytes.push(b'\n');
        elyvon_core::fs::atomic_write(&dir.join(SETTINGS_FILE), &bytes)
    }
}

fn corrupt_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".corrupt");
    p.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.updates.check_on_startup = false;
        s.layout.bottom_tab = "pianoRoll".into();
        s.add_recent(Path::new("/a.elyvon"));
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
    }

    #[test]
    fn partial_files_fill_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"{"updates":{"checkOnStartup":false}}"#,
        )
        .unwrap();
        let s = Settings::load(dir.path());
        assert!(!s.updates.check_on_startup);
        assert_eq!(s.layout, Layout::default());
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE), b"{nope").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        assert!(dir.path().join("settings.json.corrupt").exists());
    }

    #[test]
    fn sanitize_clamps_layout_and_recent() {
        let mut s = Settings::default();
        s.layout.inspector_width = 5000.0;
        s.layout.bottom_height = f64::NAN;
        s.layout.bottom_tab = "bogus".into();
        for i in 0..20 {
            s.recent_projects.push(format!("/p{i}.elyvon"));
        }
        let s = s.sanitized();
        assert_eq!(s.layout.inspector_width, 480.0);
        assert_eq!(s.layout.bottom_height, Layout::default().bottom_height);
        assert_eq!(s.layout.bottom_tab, "mixer");
        assert_eq!(s.recent_projects.len(), MAX_RECENT);
        let mut s = Settings::default();
        s.audio.buffer_frames = 3;
        s.audio.output_device = Some(String::new());
        let s = s.sanitized();
        assert_eq!(s.audio.buffer_frames, 32);
        assert_eq!(s.audio.output_device, None);
    }

    #[test]
    fn add_recent_moves_to_front_without_duplicates() {
        let mut s = Settings::default();
        s.add_recent(Path::new("/a"));
        s.add_recent(Path::new("/b"));
        s.add_recent(Path::new("/a"));
        assert_eq!(s.recent_projects, ["/a", "/b"]);
    }
}
