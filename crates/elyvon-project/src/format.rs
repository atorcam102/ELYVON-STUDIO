//! The `.elyvon` project file format.
//!
//! A project file is UTF-8 JSON with a small envelope:
//!
//! ```json
//! {
//!   "format": "elyvon-studio-project",
//!   "schemaVersion": 1,
//!   "savedWith": "0.1.0",
//!   "project": { … }
//! }
//! ```
//!
//! * `format` identifies the file independently of its extension.
//! * `schemaVersion` is bumped on every incompatible change to `project`.
//!   Older files are migrated in memory by [`migrate`]; newer files are
//!   refused with a clear message instead of being silently misread.
//! * `savedWith` records the application version for diagnostics only.
//!
//! The full specification lives in `docs/PROJECT_FORMAT.md`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ProjectError;
use crate::model::Project;
use crate::validate::validate;

pub const FORMAT_ID: &str = "elyvon-studio-project";
pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const FILE_EXTENSION: &str = "elyvon";
/// Projects reference media instead of embedding it, so 64 MiB is generous.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvelopeOut<'a> {
    format: &'static str,
    schema_version: u32,
    saved_with: &'a str,
    project: &'a Project,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnvelopeIn {
    project: Project,
}

/// Serialises a project to the current schema (pretty-printed for diffability).
pub fn encode(project: &Project, app_version: &str) -> Vec<u8> {
    let envelope = EnvelopeOut {
        format: FORMAT_ID,
        schema_version: CURRENT_SCHEMA_VERSION,
        saved_with: app_version,
        project,
    };
    // Serialising plain data structures with string keys cannot fail.
    let mut out = serde_json::to_vec_pretty(&envelope).unwrap_or_default();
    out.push(b'\n');
    out
}

/// Parses, migrates and validates a project file.
pub fn decode(bytes: &[u8]) -> Result<Project, ProjectError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| ProjectError::Syntax {
        line: e.line(),
        column: e.column(),
        message: e.to_string(),
    })?;
    let value = upgrade_envelope(value)?;
    let envelope: EnvelopeIn =
        serde_json::from_value(value).map_err(|e| ProjectError::Structure(e.to_string()))?;
    validate(&envelope.project)?;
    Ok(envelope.project)
}

/// Checks the envelope and migrates the document to [`CURRENT_SCHEMA_VERSION`].
fn upgrade_envelope(value: Value) -> Result<Value, ProjectError> {
    let obj = value.as_object().ok_or(ProjectError::NotAProject)?;
    if obj.get("format").and_then(Value::as_str) != Some(FORMAT_ID) {
        return Err(ProjectError::NotAProject);
    }
    let version = obj
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| ProjectError::Structure("missing or invalid schemaVersion".into()))?;
    let version = u32::try_from(version).map_err(|_| ProjectError::NewerSchema {
        found: u32::MAX,
        supported: CURRENT_SCHEMA_VERSION,
    })?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(ProjectError::NewerSchema {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    migrate(value, version)
}

/// Applies schema migrations `from → from + 1 → … → CURRENT`.
///
/// Each step is a pure function on the JSON document so old files never need
/// old Rust types. Add a new arm here whenever the schema version is bumped,
/// together with a fixture in `tests/fixtures/`.
pub fn migrate(value: Value, from: u32) -> Result<Value, ProjectError> {
    match from {
        CURRENT_SCHEMA_VERSION => Ok(value),
        // Example for the future:
        // 1 => migrate(v1_to_v2(value)?, 2),
        other => Err(ProjectError::UnsupportedSchema { found: other }),
    }
}

/// Reads and decodes a project file from disk.
pub fn load(path: &Path) -> Result<Project, ProjectError> {
    let bytes = elyvon_core::fs::read_limited(path, MAX_FILE_BYTES).map_err(|source| {
        ProjectError::Read {
            path: path.to_owned(),
            source,
        }
    })?;
    decode(&bytes)
}

/// Validates and atomically writes a project file.
pub fn save(path: &Path, project: &Project, app_version: &str) -> Result<(), ProjectError> {
    validate(project)?;
    let bytes = encode(project, app_version);
    elyvon_core::fs::atomic_write(path, &bytes).map_err(|source| ProjectError::Write {
        path: path.to_owned(),
        source,
    })
}

/// Appends the `.elyvon` extension when the path has none or a different one.
pub fn with_project_extension(path: &Path) -> std::path::PathBuf {
    match path.extension() {
        Some(ext) if ext.eq_ignore_ascii_case(FILE_EXTENSION) => path.to_owned(),
        _ => {
            let mut p = path.as_os_str().to_owned();
            p.push(".");
            p.push(FILE_EXTENSION);
            p.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extension_is_appended_once() {
        assert_eq!(
            with_project_extension(Path::new("a/song")),
            PathBuf::from("a/song.elyvon")
        );
        assert_eq!(
            with_project_extension(Path::new("song.ELYVON")),
            PathBuf::from("song.ELYVON")
        );
        assert_eq!(
            with_project_extension(Path::new("song.json")),
            PathBuf::from("song.json.elyvon")
        );
    }

    #[test]
    fn rejects_non_project_json() {
        assert!(matches!(decode(b"[1,2,3]"), Err(ProjectError::NotAProject)));
        assert!(matches!(
            decode(br#"{"format":"other"}"#),
            Err(ProjectError::NotAProject)
        ));
        assert!(matches!(decode(b"{oops"), Err(ProjectError::Syntax { .. })));
    }

    #[test]
    fn rejects_missing_or_unknown_versions() {
        let missing = br#"{"format":"elyvon-studio-project","project":{}}"#;
        assert!(matches!(decode(missing), Err(ProjectError::Structure(_))));
        let zero = br#"{"format":"elyvon-studio-project","schemaVersion":0,"project":{}}"#;
        assert!(matches!(
            decode(zero),
            Err(ProjectError::UnsupportedSchema { found: 0 })
        ));
        let newer = br#"{"format":"elyvon-studio-project","schemaVersion":99,"project":{}}"#;
        assert!(matches!(
            decode(newer),
            Err(ProjectError::NewerSchema { found: 99, .. })
        ));
    }
}
