// Application-level data loaded from the native side: preferences and build
// information. Audio engine state lives in ./audio.ts.

import { create } from "zustand";
import type { AppInfo } from "../bindings/AppInfo";
import type { Settings } from "../bindings/Settings";
import { ipc } from "../lib/ipc";
import { useAudio } from "./audio";
import { notifyError } from "./notifications";
import { useUi } from "./ui";

interface AppState {
  settings: Settings | null;
  info: AppInfo | null;
  load: () => Promise<void>;
  /** Updates preferences locally and persists them (debounced). */
  updateSettings: (patch: (s: Settings) => Settings) => void;
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

function persist(settings: Settings) {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    ipc.settings.set(settings).catch((e) => notifyError(e, "Could not save preferences"));
  }, 400);
}

export const useApp = create<AppState>((set, get) => ({
  settings: null,
  info: null,
  load: async () => {
    const [settings, info] = await Promise.all([ipc.settings.get(), ipc.app.info(), useAudio.getState().load()]);
    set({ settings, info });
    useUi.getState().replaceLayout(settings.layout);
  },
  updateSettings: (patch) => {
    const current = get().settings;
    if (!current) return;
    const next = patch(current);
    set({ settings: next });
    persist(next);
  },
}));

// Panel layout lives in the UI store for fast updates; mirror it into the
// persisted preferences whenever it changes.
useUi.subscribe((state, prev) => {
  if (state.layout !== prev.layout) {
    useApp.getState().updateSettings((s) => ({ ...s, layout: state.layout }));
  }
});
