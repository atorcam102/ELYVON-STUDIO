//! Bounded snapshot-based undo/redo.
//!
//! Projects hold descriptions, not audio, so a snapshot is small (kilobytes for
//! typical songs). Snapshots make undo trivially correct for every command,
//! including future ones, without hand-written inverse operations.

use std::collections::VecDeque;

use crate::model::{Project, RegionContent};

/// Approximate heap bytes held by a project snapshot (for the history budget).
pub fn approx_bytes(project: &Project) -> usize {
    let mut n = 4096 + project.media.len() * 256 + project.markers.len() * 96;
    for t in &project.tracks {
        n += 1024 + t.regions.len() * 320;
        for r in &t.regions {
            n += r.midi_events.len() * 48;
            if let RegionContent::Midi { notes } = &r.content {
                n += notes.len() * std::mem::size_of::<crate::model::MidiNote>();
            }
        }
    }
    n + crate::automation::approx_bytes(project)
}

/// One restorable state. `state_id` identifies the project content so the
/// session can tell when undo returns to the last saved state.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub project: Project,
    pub state_id: u64,
    pub label: &'static str,
}

impl Snapshot {
    fn bytes(&self) -> usize {
        approx_bytes(&self.project)
    }
}

#[derive(Debug)]
pub struct History {
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    limit: usize,
    /// Approximate bytes of all kept snapshots and the cap on them: large
    /// projects keep fewer steps instead of exhausting memory.
    bytes: usize,
    budget: usize,
}

impl History {
    pub const DEFAULT_LIMIT: usize = 200;
    /// 512 MiB of snapshots on 64-bit targets, 96 MiB on 32-bit ones.
    pub const DEFAULT_BUDGET: usize = if cfg!(target_pointer_width = "64") {
        512 << 20
    } else {
        96 << 20
    };

    pub fn new(limit: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            limit: limit.max(1),
            bytes: 0,
            budget: Self::DEFAULT_BUDGET,
        }
    }

    /// A history capped at `budget` approximate bytes (at least one step).
    pub fn with_budget(limit: usize, budget: usize) -> Self {
        Self {
            budget,
            ..Self::new(limit)
        }
    }

    /// Approximate bytes held by the kept snapshots.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Records the state *before* an edit. Clears the redo stack.
    pub fn record(&mut self, before: Snapshot) {
        for s in self.redo.drain(..) {
            self.bytes = self.bytes.saturating_sub(s.bytes());
        }
        self.bytes += before.bytes();
        self.undo.push_back(before);
        while self.undo.len() > self.limit || (self.bytes > self.budget && self.undo.len() > 1) {
            if let Some(old) = self.undo.pop_front() {
                self.bytes = self.bytes.saturating_sub(old.bytes());
            }
        }
    }

    /// Swaps `current` with the previous state. Returns `None` when empty.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.undo.pop_back()?;
        self.bytes = self.bytes + current.bytes() - prev.bytes().min(self.bytes + current.bytes());
        self.redo.push(current);
        Some(prev)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.bytes = self.bytes + current.bytes() - next.bytes().min(self.bytes + current.bytes());
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
        self.bytes = 0;
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
