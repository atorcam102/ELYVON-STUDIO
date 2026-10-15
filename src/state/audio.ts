// Audio engine state mirrored from the native core. The engine itself (device
// streams, transport clock, meters) runs in Rust; this store only reflects it.
//
// * Status arrives as `audio://status` events whenever it changes.
// * Telemetry (meters, transport position) is polled only while something is
//   audible or a view asks for live meters, so an idle engine costs nothing.

import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import type { AudioSetup } from "../bindings/AudioSetup";
import type { DeviceList } from "../bindings/DeviceList";
import type { EngineStatus } from "../bindings/EngineStatus";
import type { TelemetrySnapshot } from "../bindings/TelemetrySnapshot";
import type { TestTone } from "../bindings/TestTone";
import { inTauri, ipc, toIpcError, type IpcError } from "../lib/ipc";
import { notifyError } from "./notifications";
import { useUi } from "./ui";

export const AUDIO_STATUS_EVENT = "audio://status";
const POLL_MS = 50;
const PEAK_HOLD_MS = 1500;

interface AudioState {
  status: EngineStatus | null;
  telemetry: TelemetrySnapshot | null;
  devices: DeviceList | null;
  devicesLoading: boolean;
  devicesError: IpcError | null;
  /** Requests applying a configuration (device open can take seconds). */
  configuring: boolean;
  tone: TestTone | null;
  /**
   * What the user last asked the transport to do. The engine applies commands
   * on its next callback, so telemetry can lag the request by one buffer.
   */
  transportIntent: "playing" | "stopped";
  /** Highest output peak of the last 1.5 s (dBFS), for the peak display. */
  outputHoldDb: number;
  /** Views that need live meters even when nothing plays (e.g. input level). */
  meterViews: number;
  load: () => Promise<void>;
  refreshDevices: () => Promise<void>;
  configure: (setup: AudioSetup) => Promise<EngineStatus | null>;
  reconnect: () => Promise<void>;
  play: () => Promise<void>;
  stop: () => Promise<void>;
  togglePlay: () => Promise<void>;
  setTone: (tone: TestTone) => Promise<void>;
  watchMeters: () => () => void;
}

export function engineRunning(status: EngineStatus | null): boolean {
  return status?.state === "running";
}

let unlisten: Promise<() => void> | null = null;

export const useAudio = create<AudioState>((set, get) => ({
  status: null,
  telemetry: null,
  devices: null,
  devicesLoading: false,
  devicesError: null,
  configuring: false,
  tone: null,
  transportIntent: "stopped",
  outputHoldDb: -96,
  meterViews: 0,

  load: async () => {
    if (inTauri && !unlisten) {
      unlisten = listen<EngineStatus>(AUDIO_STATUS_EVENT, (e) => applyStatus(e.payload));
    }
    applyStatus(await ipc.audio.status());
  },

  refreshDevices: async () => {
    set({ devicesLoading: true, devicesError: null });
    try {
      set({ devices: await ipc.audio.devices() });
    } catch (e) {
      set({ devicesError: toIpcError(e) });
    } finally {
      set({ devicesLoading: false });
    }
  },

  configure: async (setup) => {
    set({ configuring: true });
    try {
      const status = await ipc.audio.configure(setup);
      // Every reopen starts with the transport stopped and the tone off.
      useAudio.setState({ tone: null, transportIntent: "stopped" });
      applyStatus(status);
      return status;
    } catch (e) {
      notifyError(e, "Could not apply the audio settings");
      return null;
    } finally {
      set({ configuring: false });
    }
  },

  reconnect: async () => {
    set({ configuring: true });
    try {
      const status = await ipc.audio.reconnect();
      useAudio.setState({ tone: null, transportIntent: "stopped" });
      applyStatus(status);
    } catch (e) {
      notifyError(e, "Could not reopen the audio device");
    } finally {
      set({ configuring: false });
    }
  },

  play: async () => {
    if (!engineRunning(get().status)) return;
    try {
      await ipc.audio.play(useUi.getState().cursorTicks);
      set({ transportIntent: "playing" });
    } catch (e) {
      notifyError(e, "Could not start playback");
    }
  },

  stop: async () => {
    if (!engineRunning(get().status)) return;
    try {
      await ipc.audio.stop();
      // Polling continues until the engine reports the stop, then the cursor
      // settles where playback ended.
      set({ transportIntent: "stopped" });
    } catch (e) {
      notifyError(e, "Could not stop playback");
    }
  },

  togglePlay: async () => (get().transportIntent === "playing" ? get().stop() : get().play()),

  setTone: async (tone) => {
    try {
      const applied = await ipc.audio.testTone(tone);
      set({ tone: applied });
      await pollOnce();
    } catch (e) {
      notifyError(e, "Could not change the test tone");
    }
  },

  watchMeters: () => {
    set((s) => ({ meterViews: s.meterViews + 1 }));
    return () => set((s) => ({ meterViews: Math.max(0, s.meterViews - 1) }));
  },
}));

function applyStatus(status: EngineStatus) {
  const current = useAudio.getState().status;
  if (current && status.revision < current.revision && status.state === current.state) return;
  const reopened = !engineRunning(status) || status.output?.deviceId !== current?.output?.deviceId;
  useAudio.setState({
    status,
    // A reopened stream always starts stopped with the tone off.
    ...(reopened ? { tone: null, transportIntent: "stopped" as const } : {}),
    ...(!engineRunning(status) ? { telemetry: null } : {}),
  });
}

// ---- telemetry polling ----

let holdAt = 0;

/** Last playhead the engine reported, to tell engine moves from user moves. */
let playheadTicks: number | null = null;

async function pollOnce() {
  if (!engineRunning(useAudio.getState().status)) return;
  try {
    const telemetry = await ipc.audio.telemetry();
    const now = Date.now();
    const peak = telemetry.outputPeakDb.length ? Math.max(...telemetry.outputPeakDb) : -96;
    const { outputHoldDb } = useAudio.getState();
    const hold = peak >= outputHoldDb || now - holdAt > PEAK_HOLD_MS ? peak : outputHoldDb;
    if (hold !== outputHoldDb || peak >= outputHoldDb) holdAt = now;
    useAudio.setState({ telemetry, outputHoldDb: hold });
    if (telemetry.playing && telemetry.positionTicks !== null) {
      playheadTicks = telemetry.positionTicks;
      useUi.getState().setCursor(telemetry.positionTicks);
    } else if (!telemetry.playing && playheadTicks !== null && telemetry.positionTicks !== null) {
      // Leave the cursor where playback stopped.
      useUi.getState().setCursor(telemetry.positionTicks);
      playheadTicks = null;
    }
  } catch {
    // Status events report real failures; a missed frame is harmless.
  }
}

function wantsPolling(s: AudioState): boolean {
  if (!engineRunning(s.status)) return false;
  return (
    s.meterViews > 0 || s.transportIntent === "playing" || !!s.telemetry?.playing || !!s.telemetry?.toneActive || !!s.tone?.enabled
  );
}

let timer: ReturnType<typeof setTimeout> | undefined;
let polling = false;

async function pollLoop() {
  if (!wantsPolling(useAudio.getState())) {
    polling = false;
    return;
  }
  await pollOnce();
  timer = setTimeout(() => void pollLoop(), POLL_MS);
}

useAudio.subscribe((s) => {
  if (wantsPolling(s) && !polling) {
    polling = true;
    clearTimeout(timer);
    void pollLoop();
  }
});

// Moving the cursor while playing relocates the engine (ruler click, Home).
useUi.subscribe((s, prev) => {
  if (s.cursorTicks === prev.cursorTicks) return;
  const { telemetry, status } = useAudio.getState();
  if (!engineRunning(status) || !telemetry?.playing) return;
  if (s.cursorTicks === playheadTicks) return;
  playheadTicks = s.cursorTicks;
  ipc.audio.locate(s.cursorTicks).catch((e) => notifyError(e, "Could not move the playhead"));
});
