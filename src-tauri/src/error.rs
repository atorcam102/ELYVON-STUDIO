//! Errors returned to the UI over IPC.
//!
//! Every command returns `Result<T, IpcError>`. The `code` is stable and drives
//! UI behaviour (e.g. `unsavedChanges` opens the save prompt); `message` is a
//! human-readable explanation suitable for display.

use elyvon_project::{EditError, ProjectError};
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorCode {
    /// The operation would discard unsaved changes.
    UnsavedChanges,
    /// Save needs a file name first.
    NeedsSaveAs,
    NotFound,
    Io,
    InvalidProject,
    NewerVersion,
    InvalidEdit,
    UpdaterUnavailable,
    Network,
    NoRelease,
    UpdateRejected,
    Cancelled,
    Busy,
    /// The audio engine is not running.
    AudioUnavailable,
    /// An audio file could not be read or decoded.
    Media,
    Internal,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
}

impl IpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn unsaved() -> Self {
        Self::new(
            ErrorCode::UnsavedChanges,
            "The project has unsaved changes. Save or discard them first.",
        )
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl From<ProjectError> for IpcError {
    fn from(e: ProjectError) -> Self {
        let code = match &e {
            ProjectError::Read { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
                ErrorCode::NotFound
            }
            ProjectError::Read { .. } => ErrorCode::Io,
            ProjectError::Write { path, .. } if path.as_os_str().is_empty() => {
                ErrorCode::NeedsSaveAs
            }
            ProjectError::Write { .. } => ErrorCode::Io,
            ProjectError::NewerSchema { .. } => ErrorCode::NewerVersion,
            ProjectError::Syntax { .. }
            | ProjectError::NotAProject
            | ProjectError::UnsupportedSchema { .. }
            | ProjectError::Structure(_)
            | ProjectError::Invalid(_) => ErrorCode::InvalidProject,
        };
        Self::new(code, e.to_string())
    }
}

impl From<EditError> for IpcError {
    fn from(e: EditError) -> Self {
        Self::new(ErrorCode::InvalidEdit, e.to_string())
    }
}

impl From<tauri::Error> for IpcError {
    fn from(e: tauri::Error) -> Self {
        Self::internal(e.to_string())
    }
}

pub type IpcResult<T> = Result<T, IpcError>;
