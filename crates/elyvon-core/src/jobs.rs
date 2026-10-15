//! Versioned background-work messages. No scheduler, worker or persistence yet.
//! Cancellation is a request until a worker reports Cancelled; UI must not infer
//! success from progress. Update downloads keep their existing DownloadEvent API.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const JOB_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum JobState {
    Unavailable {
        reason: String,
    },
    Queued,
    /// Work units, not bytes; unknown totals use None. Completion is explicit.
    Running {
        completed: u32,
        total: Option<u32>,
    },
    Completed,
    Cancelled,
    Failed {
        code: String,
        message: String,
    },
}

/// A process-scoped opaque id (never a path) and the latest observed work state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobSnapshot {
    pub contract_version: u32,
    pub id: String,
    pub state: JobState,
}

impl JobSnapshot {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contract_version != JOB_CONTRACT_VERSION {
            return Err("unsupported job contract version");
        }
        if self.id.is_empty() || self.id.len() > 128 || self.id.chars().any(char::is_control) {
            return Err("invalid job id");
        }
        if let JobState::Running {
            completed,
            total: Some(total),
        } = self.state
            && completed > total
        {
            return Err("job progress exceeds total");
        }
        Ok(())
    }
}

/// Versions advertised by app_contracts. Additive IPC types do not change the
/// on-disk project schema. Breaking changes require a version bump and fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ContractVersions {
    pub ipc: u32,
    pub project: u32,
    pub audio: u32,
    pub jobs: u32,
}

/// 2: `audio_status` returns the Phase 03 `EngineStatus`; audio commands added.
pub const IPC_CONTRACT_VERSION: u32 = 2;
