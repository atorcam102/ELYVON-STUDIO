// Typed bridge to the native core. Every call goes through `call`, which
// normalises errors into `IpcError` objects with a stable `code`.

import { Channel, invoke } from "@tauri-apps/api/core";
import type { ApplyResult } from "../bindings/ApplyResult";
import type { AudioSetup } from "../bindings/AudioSetup";
import type { DeviceList } from "../bindings/DeviceList";
import type { TelemetrySnapshot } from "../bindings/TelemetrySnapshot";
import type { TestTone } from "../bindings/TestTone";
import type { ContractVersions } from "../bindings/ContractVersions";
import type { AppInfo } from "../bindings/AppInfo";
import type { DirListing } from "../bindings/DirListing";
import type { DownloadEvent } from "../bindings/DownloadEvent";
import type { EditCommand } from "../bindings/EditCommand";
import type { EngineStatus } from "../bindings/EngineStatus";
import type { InstallResult } from "../bindings/InstallResult";
import type { IpcError } from "../bindings/IpcError";
import type { Place } from "../bindings/Place";
import type { RecoveryInfo } from "../bindings/RecoveryInfo";
import type { SessionView } from "../bindings/SessionView";
import type { Settings } from "../bindings/Settings";
import type { UpdateCheckResult } from "../bindings/UpdateCheckResult";

export type { IpcError };

export function isIpcError(e: unknown): e is IpcError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

/** Converts anything thrown by `invoke` into an `IpcError`. */
export function toIpcError(e: unknown): IpcError {
  if (isIpcError(e)) return e;
  return { code: "internal", message: e instanceof Error ? e.message : String(e) };
}

/** True when running inside the Tauri webview (false in a plain browser). */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toIpcError(e);
  }
}

export const ipc = {
  project: {
    get: () => call<SessionView>("project_get"),
    new: (discardChanges = false) => call<SessionView>("project_new", { discardChanges }),
    openDemo: (discardChanges = false) => call<SessionView>("project_open_demo", { discardChanges }),
    open: (path: string, discardChanges = false) =>
      call<SessionView>("project_open", { path, discardChanges }),
    save: (path?: string) => call<SessionView>("project_save", { path: path ?? null }),
    apply: (command: EditCommand) => call<ApplyResult>("project_apply", { command }),
    undo: () => call<SessionView>("project_undo"),
    redo: () => call<SessionView>("project_redo"),
  },
  recovery: {
    check: () => call<RecoveryInfo | null>("recovery_check"),
    restore: () => call<SessionView>("recovery_restore"),
    discard: () => call<void>("recovery_discard"),
    snapshot: () => call<void>("recovery_snapshot"),
  },
  browser: {
    list: (path?: string) => call<DirListing>("browser_list", { path: path ?? null }),
    places: () => call<Place[]>("browser_places"),
  },
  app: {
    info: () => call<AppInfo>("app_info"),
    contracts: () => call<ContractVersions>("app_contracts"),
    quit: (discardChanges = false) => call<void>("app_quit", { discardChanges }),
    restart: () => call<void>("app_restart"),
  },
  audio: {
    status: () => call<EngineStatus>("audio_status"),
    devices: () => call<DeviceList>("audio_devices"),
    configure: (setup: AudioSetup) => call<EngineStatus>("audio_configure", { setup }),
    reconnect: () => call<EngineStatus>("audio_reconnect"),
    telemetry: () => call<TelemetrySnapshot>("audio_telemetry"),
    play: (fromTicks?: number) => call<void>("audio_play", { fromTicks: fromTicks ?? null }),
    stop: () => call<void>("audio_stop"),
    locate: (ticks: number) => call<void>("audio_locate", { ticks }),
    testTone: (tone: TestTone) => call<TestTone>("audio_test_tone", { tone }),
  },
  settings: {
    get: () => call<Settings>("settings_get"),
    set: (settings: Settings) => call<Settings>("settings_set", { settings }),
  },
  updates: {
    check: () => call<UpdateCheckResult>("update_check"),
    download: (onEvent: (e: DownloadEvent) => void) => {
      const channel = new Channel<DownloadEvent>();
      channel.onmessage = onEvent;
      return call<void>("update_download", { onEvent: channel });
    },
    cancel: () => call<boolean>("update_cancel"),
    install: () => call<InstallResult>("update_install"),
  },
};
