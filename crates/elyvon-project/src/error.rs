use std::path::PathBuf;

use crate::model::{RegionId, TrackId};

/// A project failed a semantic check (bad value, broken reference, limit).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{path}: {message}")]
pub struct ValidationError {
    /// Location of the offending value, e.g. `tracks[2].regions[0].length`.
    pub path: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Loading or saving a project failed.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("the file is not valid JSON (line {line}, column {column}): {message}")]
    Syntax {
        line: usize,
        column: usize,
        message: String,
    },

    #[error("the file is not an Elyvon Studio project")]
    NotAProject,

    #[error(
        "the project was saved by a newer version of Elyvon Studio (schema {found}, this version supports up to {supported}); please update the application"
    )]
    NewerSchema { found: u32, supported: u32 },

    #[error("unsupported project schema version {found}")]
    UnsupportedSchema { found: u32 },

    #[error("the project structure is invalid: {0}")]
    Structure(String),

    #[error("the project contains invalid data: {0}")]
    Invalid(#[from] ValidationError),
}

/// An edit command could not be applied. The project is left unchanged.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    #[error("track {0} does not exist")]
    UnknownTrack(TrackId),

    #[error("region {0} does not exist")]
    UnknownRegion(RegionId),

    #[error("{0}")]
    InvalidValue(String),

    #[error("{0}")]
    Incompatible(String),

    #[error("limit reached: {0}")]
    Limit(String),

    #[error("the edit would leave the project invalid: {0}")]
    Validation(#[from] ValidationError),
}
