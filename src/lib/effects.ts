// Native effects (Phases 12–13): typed IPC wrappers, the parameter catalogue
// cache and pure helpers shared by the effect editor and its tests. All DSP,
// presets and responses come from Rust; this module only maps values to
// controls.

import { invoke } from "@tauri-apps/api/core";
import type { ChannelRef } from "../bindings/ChannelRef";
import type { EffectDetails } from "../bindings/EffectDetails";
import type { EffectInfo } from "../bindings/EffectInfo";
import type { EffectKind } from "../bindings/EffectKind";
import type { EffectParamInfo } from "../bindings/EffectParamInfo";
import type { EffectSlot } from "../bindings/EffectSlot";
import type { EqResponse } from "../bindings/EqResponse";
import type { PresetEntry } from "../bindings/PresetEntry";
import type { Spectrum } from "../bindings/Spectrum";
import { inTauri, toIpcError } from "./ipc";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toIpcError(e);
  }
}

export const effectsIpc = {
  catalog: () => call<EffectInfo[]>("effects_catalog"),
  presets: (kind: EffectKind) => call<PresetEntry[]>("effect_presets", { kind }),
  savePreset: (kind: EffectKind, name: string, params: Record<string, number>) =>
    call<PresetEntry>("effect_preset_save", { kind, name, params }),
  deletePreset: (kind: EffectKind, name: string) => call<void>("effect_preset_delete", { kind, name }),
  eqResponse: (params: Record<string, number>, points: number) => call<EqResponse>("effect_eq_response", { params, points }),
  details: (slot: EffectSlot) => call<EffectDetails>("effect_details", { slot }),
  analyzerSelect: (channel: ChannelRef | null) => call<void>("analyzer_select", { channel }),
  analyzerSpectrum: () => call<Spectrum | null>("analyzer_spectrum"),
};

let catalog: EffectInfo[] = [];
let loading: Promise<EffectInfo[]> | null = null;

/** Loads (once) and returns the effect catalogue. Empty outside Tauri. */
export function loadCatalog(): Promise<EffectInfo[]> {
  if (!inTauri) return Promise.resolve([]);
  if (catalog.length) return Promise.resolve(catalog);
  loading ??= effectsIpc
    .catalog()
    .then((c) => (catalog = c))
    .catch(() => {
      loading = null;
      return [];
    });
  return loading;
}

export function cachedCatalog(): EffectInfo[] {
  return catalog;
}

/** For tests: installs a catalogue without IPC. */
export function setCatalogForTests(c: EffectInfo[]) {
  catalog = c;
}

export function effectInfo(kind: EffectKind): EffectInfo | undefined {
  return catalog.find((e) => e.kind === kind);
}

/** Default parameter values of `kind`, keyed. */
export function defaultParams(info: EffectInfo): Record<string, number> {
  return Object.fromEntries(info.params.map((p) => [p.key, p.default]));
}

/** A new effect slot with default values. */
export function newSlot(info: EffectInfo): EffectSlot {
  return { kind: info.kind, params: defaultParams(info) };
}

/** Value of a parameter, falling back to its default. */
export function paramValue(slot: EffectSlot, p: EffectParamInfo): number {
  const v = slot.params[p.key];
  return typeof v === "number" && Number.isFinite(v) ? v : p.default;
}

export function clampParam(p: EffectParamInfo, v: number): number {
  if (!Number.isFinite(v)) return p.default;
  const c = Math.min(p.max, Math.max(p.min, v));
  return p.scale === "choice" || p.scale === "toggle" ? Math.round(c) : c;
}

/** Control position 0…1 of a value (logarithmic parameters on a log axis). */
export function toPosition(p: EffectParamInfo, v: number): number {
  const c = clampParam(p, v);
  if (p.scale === "log") return Math.log(c / p.min) / Math.log(p.max / p.min);
  return (c - p.min) / (p.max - p.min);
}

export function fromPosition(p: EffectParamInfo, pos: number): number {
  const t = Math.min(1, Math.max(0, pos));
  const v = p.scale === "log" ? p.min * Math.pow(p.max / p.min, t) : p.min + t * (p.max - p.min);
  return clampParam(p, roundForDisplay(p, v));
}

/** Rounds to the precision the control shows, so commits are stable. */
export function roundForDisplay(p: EffectParamInfo, v: number): number {
  const d = decimals(p, v);
  const k = Math.pow(10, d);
  return Math.round(v * k) / k;
}

function decimals(p: EffectParamInfo, v: number): number {
  switch (p.unit) {
    case "hz":
      return v < 100 ? 1 : 0;
    case "ms":
      return v < 10 ? 2 : v < 100 ? 1 : 0;
    case "seconds":
    case "ratio":
    case "none":
      return 2;
    case "db":
    case "semitones":
    case "bits":
      return 1;
    case "factor":
      return 1;
    default:
      return 0;
  }
}

const UNIT: Record<string, string> = {
  none: "",
  db: "dB",
  hz: "Hz",
  ms: "ms",
  seconds: "s",
  percent: "%",
  ratio: ":1",
  semitones: "st",
  cents: "ct",
  degrees: "°",
  bits: "bits",
  factor: "×",
};

/** Human-readable value with its unit (Spanish decimal comma). */
export function formatParam(p: EffectParamInfo, v: number): string {
  if (p.scale === "toggle") return v >= 0.5 ? "Sí" : "No";
  if (p.scale === "choice") return p.choices[Math.round(v)] ?? String(v);
  let shown = v;
  let unit = UNIT[p.unit] ?? "";
  if (p.unit === "hz" && v >= 1000) {
    shown = v / 1000;
    unit = "kHz";
  }
  const d = p.unit === "hz" && v >= 1000 ? (shown < 10 ? 2 : 1) : decimals(p, v);
  const text = shown.toFixed(d).replace(".", ",");
  return unit ? (unit === ":1" ? `${text}:1` : `${text} ${unit}`) : text;
}

/** Groups the equaliser's per-band parameters (b1On … b8Slope) by band. */
export function eqBands(info: EffectInfo): EffectParamInfo[][] {
  const bands: EffectParamInfo[][] = [];
  for (const p of info.params) {
    const m = /^b(\d)(On|Type|Freq|Gain|Q|Slope)$/.exec(p.key);
    if (!m) continue;
    const b = Number(m[1]) - 1;
    (bands[b] ??= []).push(p);
  }
  return bands;
}

/** Parameters that are not equaliser bands (output gain, and every other effect). */
export function plainParams(info: EffectInfo): EffectParamInfo[] {
  return info.params.filter((p) => !/^b\d(On|Type|Freq|Gain|Q|Slope)$/.test(p.key));
}

/** The band parameters hidden for a band type (gain on cuts, slope on bells…). */
export function bandParamVisible(type: number, key: string): boolean {
  const name = key.replace(/^b\d/, "");
  const cut = type === 0 || type === 4;
  if (name === "Gain") return type === 1 || type === 2 || type === 3;
  if (name === "Slope") return cut;
  return true;
}

export const CATEGORY_LABEL: Record<string, string> = {
  eq: "Ecualización",
  dynamics: "Dinámica",
  utility: "Utilidades",
  delay: "Delay",
  reverb: "Reverb",
  modulation: "Modulación",
  distortion: "Distorsión",
  pitch: "Tono",
};
