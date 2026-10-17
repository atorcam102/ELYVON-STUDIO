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
pub const CURRENT_SCHEMA_VERSION: u32 = 5;
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
    let (value, from) = upgrade_envelope(value)?;
    let mut envelope: EnvelopeIn =
        serde_json::from_value(value).map_err(|e| ProjectError::Structure(e.to_string()))?;
    if from < 4 {
        // Schema 3 → 4 typed step: recorded/imported note events become
        // editable notes with ids. Controller events are kept unchanged.
        crate::midi_edit::upgrade_regions(&mut envelope.project);
    }
    validate(&envelope.project)?;
    Ok(envelope.project)
}

/// Checks the envelope and migrates the document to [`CURRENT_SCHEMA_VERSION`].
/// Also returns the schema version the file was written with.
fn upgrade_envelope(value: Value) -> Result<(Value, u32), ProjectError> {
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
    migrate(value, version).map(|v| (v, version))
}

/// Applies schema migrations `from → from + 1 → … → CURRENT`.
///
/// Each step is a pure function on the JSON document so old files never need
/// old Rust types. Add a new arm here whenever the schema version is bumped,
/// together with a fixture in `tests/fixtures/`.
pub fn migrate(value: Value, from: u32) -> Result<Value, ProjectError> {
    match from {
        CURRENT_SCHEMA_VERSION => Ok(value),
        1 => migrate(v1_to_v2(value)?, 2),
        2 => {
            let mut value = value;
            value["schemaVersion"] = serde_json::json!(3);
            // Track IO has explicit neutral serde defaults; schema 2 files and
            // their source bytes stay unchanged. Schema 3 preserves routing.
            migrate(value, 3)
        }
        3 => {
            // Schema 4 adds the mixer (auxes, sends, inserts, groups,
            // folders), per-track strip routing and note ids/channels. Every
            // new field has a neutral serde default; note events are moved
            // into notes by the typed step in `decode`.
            let mut value = value;
            value["schemaVersion"] = serde_json::json!(4);
            migrate(value, 4)
        }
        4 => {
            // Schema 5 adds automation (channel and region lanes, modes),
            // comps, track freeze, reversed regions, fade shapes and linked
            // group edits. Every new field has a neutral serde default.
            let mut value = value;
            value["schemaVersion"] = serde_json::json!(5);
            Ok(value)
        }
        other => Err(ProjectError::UnsupportedSchema { found: other }),
    }
}

/// Schema 1 → 2 (Phases 04/05):
///
/// * `project.sampleRate` = 48000, `media` = [], `markers` = [].
/// * transport gains `tempoChanges`, `meterChanges`, `metronome`,
///   `countInBars`, `preRollBars` and `punch` with neutral defaults.
/// * every region gains `muted: false`.
/// * audio content `{source: {path} | null}` becomes a media-pool reference
///   (one pool entry per distinct path, `info: null` until decoded) with
///   offset 0, unity gain, no fades and no looping. No path is lost.
fn v1_to_v2(mut value: Value) -> Result<Value, ProjectError> {
    use serde_json::json;
    let structure = |m: &str| ProjectError::Structure(m.to_owned());
    let project = value
        .get_mut("project")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| structure("missing project"))?;
    project.insert("sampleRate".into(), json!(48_000));
    project.insert("markers".into(), json!([]));
    let transport = project
        .get_mut("transport")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| structure("missing transport"))?;
    transport.insert("tempoChanges".into(), json!([]));
    transport.insert("meterChanges".into(), json!([]));
    transport.insert(
        "metronome".into(),
        json!({ "enabled": false, "volumeDb": -6.0, "accent": true }),
    );
    transport.insert("countInBars".into(), json!(0));
    transport.insert("preRollBars".into(), json!(0));
    transport.insert(
        "punch".into(),
        json!({ "enabled": false, "start": 0, "end": 15360 }),
    );
    let mut media: Vec<(String, String)> = Vec::new(); // (path, id)
    let tracks = project
        .get_mut("tracks")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| structure("missing tracks"))?;
    for track in tracks {
        let Some(regions) = track.get_mut("regions").and_then(Value::as_array_mut) else {
            return Err(structure("missing regions"));
        };
        for region in regions {
            let region = region
                .as_object_mut()
                .ok_or_else(|| structure("region is not an object"))?;
            region.insert("muted".into(), json!(false));
            let Some(content) = region.get_mut("content").and_then(Value::as_object_mut) else {
                return Err(structure("missing region content"));
            };
            if content.get("type").and_then(Value::as_str) != Some("audio") {
                continue;
            }
            let path = content
                .remove("source")
                .and_then(|s| s.get("path").and_then(Value::as_str).map(str::to_owned));
            let id = path.map(|path| {
                if let Some((_, id)) = media.iter().find(|(p, _)| *p == path) {
                    id.clone()
                } else {
                    let id = uuid::Uuid::new_v4().to_string();
                    media.push((path, id.clone()));
                    id
                }
            });
            content.insert("media".into(), json!(id));
            content.insert("offset".into(), json!(0));
            content.insert("gainDb".into(), json!(0.0));
            content.insert("fadeIn".into(), json!(0));
            content.insert("fadeOut".into(), json!(0));
            content.insert("looped".into(), json!(false));
        }
    }
    let pool: Vec<Value> = media
        .into_iter()
        .map(|(path, id)| {
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| "Audio".into());
            let name: String = name
                .chars()
                .take(crate::validate::limits::MAX_NAME_CHARS)
                .collect();
            json!({ "id": id, "name": name, "path": path, "info": null })
        })
        .collect();
    project.insert("media".into(), Value::Array(pool));
    if let Some(v) = value.get_mut("schemaVersion") {
        *v = json!(2);
    }
    Ok(value)
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
