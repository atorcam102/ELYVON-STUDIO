// Mirror of the native project session.
//
// The UI never edits the project locally: it sends an `EditCommand` to Rust
// and renders the `SessionView` that comes back. Calls are serialised through
// a queue so responses are always applied in the order the edits were made.

import { create } from "zustand";
import type { EditCommand } from "../bindings/EditCommand";
import type { EditOutcome } from "../bindings/EditOutcome";
import type { Project } from "../bindings/Project";
import type { SessionView } from "../bindings/SessionView";
import { ipc, toIpcError } from "../lib/ipc";
import { notifyError } from "./notifications";

interface ProjectState {
  view: SessionView | null;
  /** Number of project calls in flight. */
  pending: number;
  setView: (view: SessionView) => void;
  /** Applies an edit; resolves to `null` (after notifying) when it is rejected. */
  apply: (command: EditCommand) => Promise<EditOutcome | null>;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
}

let queue: Promise<unknown> = Promise.resolve();

/** Runs `task` after every previously queued project call has settled. */
export function enqueue<T>(task: () => Promise<T>): Promise<T> {
  const run = queue.then(task, task);
  queue = run.catch(() => undefined);
  return run;
}

export const useProject = create<ProjectState>((set, get) => {
  const track = async <T>(task: () => Promise<T>): Promise<T> => {
    set((s) => ({ pending: s.pending + 1 }));
    try {
      return await enqueue(task);
    } finally {
      set((s) => ({ pending: s.pending - 1 }));
    }
  };

  return {
    view: null,
    pending: 0,
    setView: (view) => set({ view }),
    apply: async (command) => {
      try {
        const result = await track(() => ipc.project.apply(command));
        get().setView(result.view);
        return result.outcome;
      } catch (e) {
        notifyError(toIpcError(e));
        return null;
      }
    },
    undo: async () => {
      try {
        get().setView(await track(() => ipc.project.undo()));
      } catch (e) {
        notifyError(e);
      }
    },
    redo: async () => {
      try {
        get().setView(await track(() => ipc.project.redo()));
      } catch (e) {
        notifyError(e);
      }
    },
  };
});

/** The open project, or `null` before the first load. */
export const selectProject = (s: ProjectState): Project | null => s.view?.project ?? null;

export function currentProject(): Project | null {
  return useProject.getState().view?.project ?? null;
}
