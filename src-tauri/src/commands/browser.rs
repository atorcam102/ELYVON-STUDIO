//! File browser: directory listings for the right-hand browser panel.
//!
//! Listing runs on a blocking worker thread so slow or network drives never
//! stall the UI.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use ts_rs::TS;

use crate::error::{ErrorCode, IpcError, IpcResult};
use crate::state::AppState;

const MAX_ENTRIES: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntryKind {
    Directory,
    Project,
    Audio,
    Midi,
    Other,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DirListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<DirEntry>,
    /// More than the display limit; only the first entries are returned.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Place {
    pub label: String,
    pub path: String,
}

pub fn classify(path: &Path, is_dir: bool) -> EntryKind {
    if is_dir {
        return EntryKind::Directory;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        elyvon_project::format::FILE_EXTENSION => EntryKind::Project,
        "wav" | "wave" | "aif" | "aiff" | "flac" | "mp3" | "ogg" | "m4a" | "caf" => {
            EntryKind::Audio
        }
        "mid" | "midi" => EntryKind::Midi,
        _ => EntryKind::Other,
    }
}

pub fn list_dir(dir: &Path) -> std::io::Result<DirListing> {
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // Follows symlinks; broken links are skipped.
        let Ok(meta) = std::fs::metadata(entry.path()) else {
            continue;
        };
        if entries.len() == MAX_ENTRIES {
            truncated = true;
            break;
        }
        let path = entry.path();
        entries.push(DirEntry {
            kind: classify(&path, meta.is_dir()),
            size: meta.is_file().then_some(meta.len()),
            name,
            path: path.display().to_string(),
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Directory)
            .cmp(&(b.kind != EntryKind::Directory))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(DirListing {
        path: dir.display().to_string(),
        parent: dir.parent().map(|p| p.display().to_string()),
        entries,
        truncated,
    })
}

#[tauri::command]
pub async fn browser_list(app: AppHandle, path: Option<String>) -> IpcResult<DirListing> {
    let state = app.state::<AppState>();
    let dir: PathBuf = match path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => state
            .settings()
            .browser_dir
            .clone()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| state.paths.home_dir.clone())
            .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "No home directory available."))?,
    };
    let listing_dir = dir.clone();
    let listing = tauri::async_runtime::spawn_blocking(move || list_dir(&listing_dir))
        .await
        .map_err(IpcError::from)?
        .map_err(|e| {
            let code = if e.kind() == std::io::ErrorKind::NotFound {
                ErrorCode::NotFound
            } else {
                ErrorCode::Io
            };
            IpcError::new(code, format!("Cannot open {}: {e}", dir.display()))
        })?;
    state.settings().browser_dir = Some(listing.path.clone());
    Ok(listing)
}

#[tauri::command]
pub async fn browser_places(app: AppHandle) -> IpcResult<Vec<Place>> {
    let p = app.path();
    let candidates = [
        ("Home", p.home_dir()),
        ("Music", p.audio_dir()),
        ("Documents", p.document_dir()),
        ("Desktop", p.desktop_dir()),
        ("Downloads", p.download_dir()),
    ];
    Ok(candidates
        .into_iter()
        .filter_map(|(label, dir)| dir.ok().filter(|d| d.is_dir()).map(|d| (label, d)))
        .map(|(label, d)| Place {
            label: label.into(),
            path: d.display().to_string(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_directories_first_and_hides_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Samples")).unwrap();
        std::fs::write(dir.path().join("b.wav"), b"x").unwrap();
        std::fs::write(dir.path().join("A Song.elyvon"), b"x").unwrap();
        std::fs::write(dir.path().join(".hidden"), b"x").unwrap();
        let l = list_dir(dir.path()).unwrap();
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Samples", "A Song.elyvon", "b.wav"]);
        assert_eq!(l.entries[1].kind, EntryKind::Project);
        assert_eq!(l.entries[2].kind, EntryKind::Audio);
        assert_eq!(l.entries[2].size, Some(1));
        assert!(!l.truncated);
    }

    #[test]
    fn missing_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_dir(&dir.path().join("nope")).is_err());
    }

    #[test]
    fn classifies_extensions_case_insensitively() {
        assert_eq!(classify(Path::new("x.MID"), false), EntryKind::Midi);
        assert_eq!(classify(Path::new("x.Flac"), false), EntryKind::Audio);
        assert_eq!(classify(Path::new("x.txt"), false), EntryKind::Other);
        assert_eq!(classify(Path::new("x.wav"), true), EntryKind::Directory);
    }
}
