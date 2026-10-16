// Mirror of the native engine's transport: everything here comes from engine
// events (`audio://position`, `audio://meters`, `audio://status`,
// `media://status`). The UI never advances the playhead with its own timers.

import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import type { EngineStats } from "../bindings/EngineStats";
import type { EngineStatus } from "../bindings/EngineStatus";
import type { MediaStatus } from "../bindings/MediaStatus";
import type { Meters } from "../bindings/Meters";
import type { TransportCommand } from "../bindings/TransportCommand";
import type { TransportSnapshot } from "../bindings/TransportSnapshot";
import { inTauri, ipc } from "../lib/ipc";
import { useApp } from "./app";
import { notifyError } from "./notifications";

interface TransportState {
  snapshot: TransportSnapshot | null;
  meters: Meters | null;
  stats: EngineStats | null;
  media: Record<string, MediaStatus["state"]>;
  /** Follow the playhead by scrolling the arrangement. */
  follow: boolean;
  /** Count-in before playback (project setting `countInBars` must be > 0). */
  countIn: boolean;
  setSnapshot: (s: TransportSnapshot) => void;
  setMedia: (list: MediaStatus[]) => void;
  setFollow: (on: boolean) => void;
  setCountIn: (on: boolean) => void;
  send: (command: TransportCommand) => Promise<void>;
}

export const useTransport = create<TransportState>((set) => ({
  snapshot: null,
  meters: null,
  stats: null,
  media: {},
  follow: true,
  countIn: false,
  setSnapshot: (snapshot) => set({ snapshot }),
  setMedia: (list) => set({ media: Object.fromEntries(list.map((m) => [m.id, m.state])) }),
  setFollow: (follow) => set({ follow }),
  setCountIn: (countIn) => set({ countIn }),
  send: async (command) => {
    try {
      set({ snapshot: await ipc.transport.command(command) });
    } catch (e) {
      notifyError(e);
    }
  },
}));

export function isRolling(s: TransportSnapshot | null): boolean {
  return s?.state === "playing" || s?.state === "countIn";
}

/** Subscribes to engine events; returns an unsubscribe function. */
export function connectTransport(): () => void {
  if (!inTauri) return () => undefined;
  const unlisten = [
    listen<TransportSnapshot>("audio://position", (e) => useTransport.getState().setSnapshot(e.payload)),
    listen<[Meters, EngineStats]>("audio://meters", (e) => useTransport.setState({ meters: e.payload[0], stats: e.payload[1] })),
    listen<EngineStatus>("audio://status", (e) => useApp.setState({ engine: e.payload })),
    listen<MediaStatus[]>("media://status", (e) => useTransport.getState().setMedia(e.payload)),
  ];
  void ipc.transport.state().then((s) => s && useTransport.getState().setSnapshot(s)).catch(() => undefined);
  void ipc.media.status().then((m) => useTransport.getState().setMedia(m)).catch(() => undefined);
  void ipc.app.audioStatus().then((engine) => useApp.setState({ engine })).catch(() => undefined);
  return () => unlisten.forEach((p) => void p.then((f) => f()));
}
