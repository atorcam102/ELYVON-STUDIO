// Application-level data loaded from the native side: preferences, build
// information and audio engine status.

import { create } from "zustand";
import type { AppInfo } from "../bindings/AppInfo";
import type { EngineStatus } from "../bindings/EngineStatus";
import type { Settings } from "../bindings/Settings";
import { ipc } from "../lib/ipc";
import { notifyError } from "./notifications";
import { useUi } from "./ui";

interface AppState {
  settings: Settings | null;
  info: AppInfo | null;
  engine: EngineStatus | null;
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
  engine: null,
  load: async () => {
    const [settings, info, engine] = await Promise.all([ipc.settings.get(), ipc.app.info(), ipc.app.audioStatus()]);
    set({ settings, info, engine });
    // Migrate the old shipped dimensions, preserving custom panel sizes.
    const layout = { ...settings.layout };
    if (layout.trackHeaderWidth === 220) layout.trackHeaderWidth = 408;
    if (layout.browserWidth === 260) layout.browserWidth = 404;
    if (layout.inspectorWidth === 240) layout.inspectorWidth = 296;
    useUi.getState().replaceLayout(layout);
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

export function engineAvailable(engine: EngineStatus | null): boolean {
  return engine?.state === "running";
}
