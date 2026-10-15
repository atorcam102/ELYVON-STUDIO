//! Bounded snapshot-based undo/redo.
//!
//! Projects hold descriptions, not audio, so a snapshot is small (kilobytes for
//! typical songs). Snapshots make undo trivially correct for every command,
//! including future ones, without hand-written inverse operations.

use std::collections::VecDeque;

use crate::model::Project;

/// One restorable state. `state_id` identifies the project content so the
/// session can tell when undo returns to the last saved state.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub project: Project,
    pub state_id: u64,
    pub label: &'static str,
}

#[derive(Debug)]
pub struct History {
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    limit: usize,
}

impl History {
    pub const DEFAULT_LIMIT: usize = 200;

    pub fn new(limit: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Records the state *before* an edit. Clears the redo stack.
    pub fn record(&mut self, before: Snapshot) {
        self.redo.clear();
        if self.undo.len() == self.limit {
            self.undo.pop_front();
        }
        self.undo.push_back(before);
    }

    /// Swaps `current` with the previous state. Returns `None` when empty.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.undo.pop_back()?;
        self.redo.push(current);
        Some(prev)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push_back(current);
        Some(next)
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.back().map(|s| s.label)
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|s| s.label)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new(Self::DEFAULT_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(name: &str, id: u64) -> Snapshot {
        Snapshot {
            project: Project::new(name),
            state_id: id,
            label: "Edit",
        }
    }

    #[test]
    fn undo_redo_cycle() {
        let mut h = History::new(10);
        h.record(snap("a", 0));
        let back = h.undo(snap("b", 1)).unwrap();
        assert_eq!(back.project.name, "a");
        let fwd = h.redo(back).unwrap();
        assert_eq!(fwd.project.name, "b");
        assert!(h.redo(fwd).is_none());
    }

    #[test]
    fn record_clears_redo_and_respects_limit() {
        let mut h = History::new(2);
        h.record(snap("a", 0));
        h.record(snap("b", 1));
        h.record(snap("c", 2));
        assert_eq!(h.undo_len(), 2);
        let s = h.undo(snap("d", 3)).unwrap();
        assert_eq!(s.project.name, "c");
        h.record(s);
        assert!(h.redo_label().is_none());
    }
}
