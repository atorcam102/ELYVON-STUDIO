// Typed bridge to the native core. Every call goes through `call`, which
// normalises errors into `IpcError` objects with a stable `code`.

import { Channel, invoke } from "@tauri-apps/api/core";
import type { ApplyResult } from "../bindings/ApplyResult";
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
import type { DeviceInfo } from "../bindings/DeviceInfo";
import type { ImportLayout } from "../bindings/ImportLayout";
import type { ImportResult } from "../bindings/ImportResult";
import type { MediaStatus } from "../bindings/MediaStatus";
import type { RenderReport } from "../bindings/RenderReport";
import type { TransportCommand } from "../bindings/TransportCommand";
import type { TransportSnapshot } from "../bindings/TransportSnapshot";

import type { MidiStatus } from "../bindings/MidiStatus";
import type { LatencyMeasurement } from "../bindings/LatencyMeasurement";
import type { CaptureStatus } from "../bindings/CaptureStatus";
import type { CaptureResult } from "../bindings/CaptureResult";
import type { RecordedTake } from "../bindings/RecordedTake";
import type { MixerStatus } from "../bindings/MixerStatus";
import type { StemsReport } from "../bindings/StemsReport";
import type { AutomationParam } from "../bindings/AutomationParam";
import type { ChannelRef } from "../bindings/ChannelRef";
import type { RepairResult } from "../bindings/RepairResult";
import type { TouchPhase } from "../bindings/TouchPhase";
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
  midi: {
    status:()=>call<MidiStatus>("midi_status"),refresh:()=>call<void>("midi_refresh"),panic:()=>call<void>("midi_panic"),
    send:(trackId:string,bytes:number[])=>call<void>("midi_send",{trackId,bytes}),
    preview:(trackId:string,channel:number,pitch:number,velocity:number,durationMs:number)=>call<void>("midi_preview",{trackId,channel,pitch,velocity,durationMs}),
    recordStart:(fromTick:number)=>call<MidiStatus>("midi_record_start",{fromTick}),recordStop:(cancel=false)=>call<SessionView>("midi_record_stop",{cancel}),
    import:(path:string,startTick:number,useFileTempo:boolean)=>call<SessionView>("midi_import",{path,startTick,useFileTempo}),
    export:(path:string)=>call<string>("midi_export",{path}),
  },
  recording: {
    measure:(referencePath:string,capturedPath:string,maxDelay:number)=>call<LatencyMeasurement>("recording_measure_latency",{referencePath,capturedPath,maxDelay}),
    devices:()=>call<DeviceInfo[]>("recording_devices"),
    status:()=>call<CaptureStatus>("recording_status"),
    start:(fromTick:number,monitorOnly=false)=>call<CaptureStatus>("recording_start",{fromTick,monitorOnly}),
    stop:(cancel=false)=>call<CaptureResult>("recording_stop",{cancel}),
    recoverable:()=>call<RecordedTake[]>("recording_recoverable"),
    recover:(path:string)=>call<CaptureResult>("recording_recover",{path}),
  },
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
    audioStatus: () => call<EngineStatus>("audio_status"),
    quit: (discardChanges = false) => call<void>("app_quit", { discardChanges }),
    restart: () => call<void>("app_restart"),
  },
  audio: {
    devices: () => call<DeviceInfo[]>("audio_devices"),
    restart: () => call<EngineStatus>("audio_restart"),
  },
  transport: {
    command: (command: TransportCommand) => call<TransportSnapshot>("transport_command", { command }),
    state: () => call<TransportSnapshot | null>("transport_state"),
    tap: () => call<number | null>("transport_tap"),
  },
  media: {
    formats: () => call<string[]>("media_formats"),
    status: () => call<MediaStatus[]>("media_status"),
    import: (paths: string[], trackId: string | null, start: number, layout: ImportLayout) =>
      call<ImportResult>("media_import", { paths, trackId, start, layout }),
    relink: (id: string, path: string) => call<SessionView>("media_relink", { id, path }),
    /** Binary waveform columns; see `decodePeaks`. */
    regionPeaks: (regionId: string, fromTick: number, toTick: number, columns: number) =>
      call<ArrayBuffer>("media_region_peaks", { regionId, fromTick, toTick, columns }),
    render: (path: string, metronome: boolean, startTick?: number, endTick?: number) =>
      call<RenderReport>("project_render", { path, startTick: startTick ?? null, endTick: endTick ?? null, metronome }),
    exportStems: (directory: string) => call<StemsReport>("project_export_stems", { directory }),
  },
  automation: {
    /** A control gesture; `true` when it was written as automation. */
    touch: (channel: ChannelRef, param: AutomationParam, value: number, phase: TouchPhase) =>
      call<boolean>("automation_touch", { channel, param, value, phase }),
  },
  editing: {
    normalize: (ids: string[], targetDb: number) => call<ApplyResult>("region_normalize", { ids, targetDb }),
    transients: (id: string, sensitivity: number) => call<number[]>("region_transients", { id, sensitivity }),
    freeze: (id: string) => call<ApplyResult>("track_freeze", { id }),
    bounce: (trackId: string, regionIds: string[] | null, includeFader: boolean, muteSources: boolean) =>
      call<ApplyResult>("track_bounce", { trackId, regionIds, includeFader, muteSources }),
  },
  repair: {
    search: (directory: string) => call<RepairResult>("media_repair", { directory }),
    collect: () => call<SessionView>("media_collect"),
    unused: () => call<string[]>("media_unused"),
    backupUnused: (confirm: boolean) => call<string>("media_backup_unused", { confirm }),
    rescan: () => call<MediaStatus[]>("media_rescan"),
  },
  mixer: {
    status: () => call<MixerStatus>("mixer_status"),
    resetClips: () => call<void>("mixer_reset_clips"),
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

/** Waveform columns: `[minL, maxL, minR, maxR]` per column, in −1…1. */
export interface Peaks {
  columns: number;
  data: Float32Array;
}

/** Decodes the `media_region_peaks` wire format (u32 LE count, then i16 LE). */
export function decodePeaks(buffer: ArrayBuffer): Peaks {
  const view = new DataView(buffer);
  const columns = buffer.byteLength >= 4 ? view.getUint32(0, true) : 0;
  const n = Math.min(columns * 4, Math.floor((buffer.byteLength - 4) / 2));
  const data = new Float32Array(n);
  for (let i = 0; i < n; i++) data[i] = view.getInt16(4 + i * 2, true) / 32767;
  return { columns, data };
}
