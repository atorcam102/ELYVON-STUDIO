//! An open project together with its file path, undo history and dirty state.

use std::path::{Path, PathBuf};

use serde::Serialize;
use ts_rs::TS;

use crate::edit::{self, EditCommand, EditOutcome};
use crate::error::{EditError, ProjectError};
use crate::format;
use crate::history::{History, Snapshot};
use crate::model::Project;

/// Where the open project came from. Drives UI hints such as the demo badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ProjectOrigin {
    /// Created with File ▸ New.
    New,
    /// The bundled demonstration session (never written over a real project).
    Demo,
    /// Loaded from or saved to a file.
    File,
    /// Restored from a crash-recovery snapshot.
    Recovered,
}

/// A pending save produced by [`ProjectSession::begin_save`].
#[derive(Debug)]
pub struct SaveTicket {
    pub project: Project,
    pub path: PathBuf,
    state_id: u64,
}

#[derive(Debug)]
pub struct ProjectSession {
    project: Project,
    path: Option<PathBuf>,
    origin: ProjectOrigin,
    history: History,
    /// Identifies the current content; equal ids mean identical content.
    state_id: u64,
    /// `state_id` at the last successful save (`None` if never saved).
    saved_state_id: Option<u64>,
    next_state_id: u64,
    /// Monotonic change counter (unlike `state_id`, never goes back on undo).
    revision: u64,
}

/// Everything the UI needs to render the session, sent after every change.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionView {
    pub project: Project,
    pub path: Option<String>,
    pub origin: ProjectOrigin,
    pub dirty: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    /// Increments on every change; lets the UI discard stale responses.
    #[ts(type = "number")]
    pub revision: u64,
}

impl ProjectSession {
    fn with(project: Project, path: Option<PathBuf>, origin: ProjectOrigin, saved: bool) -> Self {
        Self {
            project,
            path,
            origin,
            history: History::default(),
            state_id: 0,
            saved_state_id: saved.then_some(0),
            next_state_id: 1,
            revision: 0,
        }
    }

    /// A new, empty project. Not dirty until the user changes something.
    pub fn new_empty() -> Self {
        Self::with(
            Project::new(Project::DEFAULT_NAME),
            None,
            ProjectOrigin::New,
            true,
        )
    }

    pub fn demo() -> Self {
        Self::with(crate::demo::demo_project(), None, ProjectOrigin::Demo, true)
    }

    pub fn open(path: &Path) -> Result<Self, ProjectError> {
        let project = format::load(path)?;
        Ok(Self::with(
            project,
            Some(path.to_owned()),
            ProjectOrigin::File,
            true,
        ))
    }

    /// A project restored after a crash: always dirty, so the user saves it.
    pub fn recovered(project: Project, original_path: Option<PathBuf>) -> Self {
        Self::with(project, original_path, ProjectOrigin::Recovered, false)
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn origin(&self) -> ProjectOrigin {
        self.origin
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_state_id != Some(self.state_id)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn view(&self) -> SessionView {
        SessionView {
            project: self.project.clone(),
            path: self.path.as_ref().map(|p| p.display().to_string()),
            origin: self.origin,
            dirty: self.is_dirty(),
            undo_label: self.history.undo_label().map(str::to_owned),
            redo_label: self.history.redo_label().map(str::to_owned),
            revision: self.revision,
        }
    }

    /// Applies an edit and records it for undo. No-op edits are not recorded.
    pub fn apply(&mut self, cmd: &EditCommand) -> Result<EditOutcome, EditError> {
        let before = self.project.clone();
        let (outcome, before) = edit::apply_with_backup(&mut self.project, cmd, before)?;
        if before != self.project {
            self.history.record(Snapshot {
                project: before,
                state_id: self.state_id,
                label: cmd.label(),
            });
            self.state_id = self.next_state_id;
            self.next_state_id += 1;
            self.revision += 1;
        }
        Ok(outcome)
    }

    pub fn undo(&mut self) -> bool {
        self.step(true)
    }

    pub fn redo(&mut self) -> bool {
        self.step(false)
    }

    fn step(&mut self, undo: bool) -> bool {
        let label = if undo {
            self.history.undo_label()
        } else {
            self.history.redo_label()
        };
        let Some(label) = label else { return false };
        let current = Snapshot {
            project: std::mem::replace(&mut self.project, Project::new(Project::DEFAULT_NAME)),
            state_id: self.state_id,
            label,
        };
        let restored = if undo {
            self.history.undo(current)
        } else {
            self.history.redo(current)
        };
        // The label check above guarantees a snapshot is available.
        if let Some(s) = restored {
            self.project = s.project;
            self.state_id = s.state_id;
        }
        self.revision += 1;
        true
    }

    /// Saves to `path` (or the current path). Marks the session clean on success.
    pub fn save(
        &mut self,
        path: Option<&Path>,
        app_version: &str,
    ) -> Result<PathBuf, ProjectError> {
        let ticket = self.begin_save(path)?;
        format::save(&ticket.path, &ticket.project, app_version)?;
        Ok(self.complete_save(ticket))
    }

    /// First half of a save: captures what to write without touching the disk,
    /// so callers can perform the IO without holding a lock on the session.
    pub fn begin_save(&self, path: Option<&Path>) -> Result<SaveTicket, ProjectError> {
        let target = match (path, &self.path) {
            (Some(p), _) => format::with_project_extension(p),
            (None, Some(p)) if !self.needs_save_as() => p.clone(),
            _ => {
                return Err(ProjectError::Write {
                    path: PathBuf::new(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "no file chosen; use Save As",
                    ),
                });
            }
        };
        Ok(SaveTicket {
            project: self.project.clone(),
            path: target,
            state_id: self.state_id,
        })
    }

    /// Second half of a save, after the ticket's project was written to disk.
    /// Edits made in the meantime keep the session dirty.
    pub fn complete_save(&mut self, ticket: SaveTicket) -> PathBuf {
        self.path = Some(ticket.path.clone());
        self.origin = ProjectOrigin::File;
        self.saved_state_id = Some(ticket.state_id);
        self.revision += 1;
        ticket.path
    }

    /// Whether Save needs a file name from the user first.
    pub fn needs_save_as(&self) -> bool {
        self.path.is_none() || matches!(self.origin, ProjectOrigin::Demo | ProjectOrigin::New)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackKind;

    fn add_track() -> EditCommand {
        EditCommand::AddTrack {
            kind: TrackKind::Audio,
            name: None,
            index: None,
        }
    }

    #[test]
    fn new_session_is_clean_and_edits_make_it_dirty() {
        let mut s = ProjectSession::new_empty();
        assert!(!s.is_dirty());
        s.apply(&add_track()).unwrap();
        assert!(s.is_dirty());
        assert_eq!(s.view().undo_label.as_deref(), Some("New Track"));
    }

    #[test]
    fn undo_back_to_saved_state_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ProjectSession::new_empty();
        s.apply(&add_track()).unwrap();
        s.save(Some(&dir.path().join("song")), "test").unwrap();
        assert!(!s.is_dirty());
        s.apply(&add_track()).unwrap();
        assert!(s.is_dirty());
        assert!(s.undo());
        assert!(!s.is_dirty(), "back at the saved content");
        assert!(s.redo());
        assert!(s.is_dirty());
        assert!(!s.redo());
    }

    #[test]
    fn failed_and_noop_edits_leave_history_untouched() {
        let mut s = ProjectSession::new_empty();
        assert!(s.apply(&EditCommand::SetTempo { bpm: 0.0 }).is_err());
        s.apply(&EditCommand::SetTempo { bpm: 120.0 }).unwrap(); // already 120
        assert!(!s.is_dirty());
        assert!(!s.undo());
    }

    #[test]
    fn demo_requires_save_as() {
        let mut s = ProjectSession::demo();
        assert!(s.needs_save_as());
        assert!(s.save(None, "test").is_err());
        let dir = tempfile::tempdir().unwrap();
        let saved = s.save(Some(&dir.path().join("demo-copy")), "test").unwrap();
        assert!(saved.ends_with("demo-copy.elyvon"));
        assert_eq!(s.origin(), ProjectOrigin::File);
        assert!(!s.needs_save_as());
    }

    #[test]
    fn edits_during_a_save_keep_the_session_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ProjectSession::new_empty();
        let ticket = s.begin_save(Some(&dir.path().join("a"))).unwrap();
        s.apply(&add_track()).unwrap();
        format::save(&ticket.path, &ticket.project, "test").unwrap();
        s.complete_save(ticket);
        assert!(s.is_dirty());
        assert!(!s.needs_save_as());
    }

    #[test]
    fn recovered_sessions_start_dirty() {
        let s = ProjectSession::recovered(Project::new("x"), None);
        assert!(s.is_dirty());
    }
}
