//! Crash-recovery snapshots.
//!
//! While a project has unsaved changes the application periodically writes a
//! snapshot to its data directory. A clean save, discard or exit deletes it, so
//! a snapshot found at start-up means the previous run ended unexpectedly.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::ProjectError;
use crate::format;
use crate::model::Project;

pub const RECOVERY_FORMAT_ID: &str = "elyvon-studio-recovery";
pub const RECOVERY_FILE_NAME: &str = "autosave.elyvon-recovery";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryFile {
    format: String,
    original_path: Option<String>,
    saved_at_unix: u64,
    /// A complete project file envelope, migrated like any other project.
    project_file: Value,
}

/// Summary shown to the user before restoring.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveryInfo {
    pub project_name: String,
    pub original_path: Option<String>,
    #[ts(type = "number")]
    pub saved_at_unix: u64,
}

pub fn recovery_path(dir: &Path) -> PathBuf {
    dir.join(RECOVERY_FILE_NAME)
}

/// Writes (atomically) the recovery snapshot for the open project.
pub fn write(
    dir: &Path,
    project: &Project,
    original_path: Option<&Path>,
    app_version: &str,
) -> Result<(), ProjectError> {
    std::fs::create_dir_all(dir).map_err(|source| ProjectError::Write {
        path: dir.to_owned(),
        source,
    })?;
    let project_file: Value = serde_json::from_slice(&format::encode(project, app_version))
        .map_err(|e| ProjectError::Structure(e.to_string()))?;
    let file = RecoveryFile {
        format: RECOVERY_FORMAT_ID.into(),
        original_path: original_path.map(|p| p.display().to_string()),
        saved_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        project_file,
    };
    let bytes = serde_json::to_vec(&file).map_err(|e| ProjectError::Structure(e.to_string()))?;
    let path = recovery_path(dir);
    elyvon_core::fs::atomic_write(&path, &bytes)
        .map_err(|source| ProjectError::Write { path, source })
}

/// Reads the snapshot, if any. Corrupt snapshots are reported as errors so the
/// caller can log and discard them.
pub fn read(dir: &Path) -> Result<Option<(RecoveryInfo, Project)>, ProjectError> {
    let path = recovery_path(dir);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = elyvon_core::fs::read_limited(&path, format::MAX_FILE_BYTES).map_err(|source| {
        ProjectError::Read {
            path: path.clone(),
            source,
        }
    })?;
    let file: RecoveryFile = serde_json::from_slice(&bytes).map_err(|e| ProjectError::Syntax {
        line: e.line(),
        column: e.column(),
        message: e.to_string(),
    })?;
    if file.format != RECOVERY_FORMAT_ID {
        return Err(ProjectError::NotAProject);
    }
    let inner = serde_json::to_vec(&file.project_file)
        .map_err(|e| ProjectError::Structure(e.to_string()))?;
    let project = format::decode(&inner)?;
    let info = RecoveryInfo {
        project_name: project.name.clone(),
        original_path: file.original_path,
        saved_at_unix: file.saved_at_unix,
    };
    Ok(Some((info, project)))
}

/// Removes the snapshot. Missing files are not an error.
pub fn clear(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(recovery_path(dir)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_clear() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path()).unwrap().is_none());
        let project = crate::demo::demo_project();
        write(
            dir.path(),
            &project,
            Some(Path::new("/music/song.elyvon")),
            "0.1.0",
        )
        .unwrap();
        let (info, restored) = read(dir.path()).unwrap().unwrap();
        assert_eq!(restored, project);
        assert_eq!(info.project_name, "Demo Session");
        assert_eq!(info.original_path.as_deref(), Some("/music/song.elyvon"));
        clear(dir.path()).unwrap();
        clear(dir.path()).unwrap();
        assert!(read(dir.path()).unwrap().is_none());
    }

    #[test]
    fn corrupt_snapshot_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(recovery_path(dir.path()), b"{garbage").unwrap();
        assert!(read(dir.path()).is_err());
    }
}
