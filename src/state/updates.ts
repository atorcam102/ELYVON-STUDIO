// Update workflow state machine. Every transition that touches the network or
// the installation is triggered by the user, except the single optional check
// at start-up (which only reports; it never downloads).

import { create } from "zustand";
import type { UpdateInfo } from "../bindings/UpdateInfo";
import { ipc, toIpcError, type IpcError } from "../lib/ipc";

export type UpdatePhase =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "unavailable"; reason: string }
  | { phase: "upToDate"; currentVersion: string }
  | { phase: "prereleaseSkipped"; currentVersion: string; version: string }
  | { phase: "available"; info: UpdateInfo }
  | { phase: "downloading"; info: UpdateInfo; downloaded: number; total: number | null }
  | { phase: "ready"; info: UpdateInfo }
  | { phase: "installing"; info: UpdateInfo }
  | { phase: "installed"; info: UpdateInfo; restartRequired: boolean }
  | { phase: "error"; error: IpcError; info?: UpdateInfo };

interface UpdateState {
  state: UpdatePhase;
  /** Last check was the silent start-up check. */
  background: boolean;
  check: (opts?: { background?: boolean }) => Promise<void>;
  download: () => Promise<void>;
  cancel: () => Promise<void>;
  install: () => Promise<void>;
  reset: () => void;
}

function infoOf(s: UpdatePhase): UpdateInfo | undefined {
  return "info" in s ? s.info : undefined;
}

export const useUpdates = create<UpdateState>((set, get) => ({
  state: { phase: "idle" },
  background: false,

  check: async ({ background = false } = {}) => {
    const busy = ["checking", "downloading", "installing"];
    if (busy.includes(get().state.phase)) return;
    set({ state: { phase: "checking" }, background });
    try {
      const r = await ipc.updates.check();
      switch (r.status) {
        case "unavailable":
          set({ state: { phase: "unavailable", reason: r.reason } });
          break;
        case "upToDate":
          set({ state: { phase: "upToDate", currentVersion: r.currentVersion } });
          break;
        case "prereleaseSkipped":
          set({ state: { phase: "prereleaseSkipped", currentVersion: r.currentVersion, version: r.version } });
          break;
        case "available":
          set({ state: { phase: "available", info: r.info } });
          break;
      }
    } catch (e) {
      set({ state: { phase: "error", error: toIpcError(e) } });
    }
  },

  download: async () => {
    const info = infoOf(get().state);
    if (!info) return;
    set({ state: { phase: "downloading", info, downloaded: 0, total: null }, background: false });
    try {
      await ipc.updates.download((event) => {
        const s = get().state;
        if (s.phase !== "downloading") return;
        if (event.event === "started") set({ state: { ...s, total: event.contentLength } });
        if (event.event === "progress")
          set({ state: { ...s, downloaded: event.downloaded, total: event.contentLength } });
      });
      set({ state: { phase: "ready", info } });
    } catch (e) {
      const error = toIpcError(e);
      set({ state: error.code === "cancelled" ? { phase: "available", info } : { phase: "error", error, info } });
    }
  },

  cancel: async () => {
    await ipc.updates.cancel().catch(() => false);
  },

  install: async () => {
    const s = get().state;
    if (s.phase !== "ready") return;
    set({ state: { phase: "installing", info: s.info } });
    try {
      const r = await ipc.updates.install();
      set({ state: { phase: "installed", info: s.info, restartRequired: r.restartRequired } });
    } catch (e) {
      const error = toIpcError(e);
      // Unsaved changes: the package is still verified and ready.
      set({ state: error.code === "unsavedChanges" ? { phase: "ready", info: s.info } : { phase: "error", error, info: s.info } });
      if (error.code === "unsavedChanges") throw error;
    }
  },

  reset: () => set({ state: { phase: "idle" }, background: false }),
}));

/** Whether the toolbar should advertise an update. */
export function hasUpdateBadge(s: UpdatePhase): boolean {
  return ["available", "downloading", "ready", "installed"].includes(s.phase);
}
