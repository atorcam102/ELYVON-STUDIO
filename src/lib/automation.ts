// Automation for drawing and display only. The native core owns lanes,
// edits and playback; this mirrors its evaluation (ticks → value) so lanes
// can be drawn and controls can show automated values while playing.
// Verified against `crates/elyvon-project/tests/fixtures/automation_golden.json`.

import type { AutomationLane } from "../bindings/AutomationLane";
import type { AutomationMode } from "../bindings/AutomationMode";
import type { AutomationParam } from "../bindings/AutomationParam";
import type { ChannelAutomation } from "../bindings/ChannelAutomation";
import type { ChannelRef } from "../bindings/ChannelRef";
import type { Curve } from "../bindings/Curve";
import type { InsertSlot } from "../bindings/InsertSlot";
import type { Project } from "../bindings/Project";
import type { Region } from "../bindings/Region";
import { formatDb, formatPan } from "./format";
import { effectInfo, formatParam } from "./effects";

export interface ParamSpec {
  min: number;
  max: number;
  log: boolean;
  discrete: boolean;
}

const linear = (min: number, max: number): ParamSpec => ({ min, max, log: false, discrete: false });
const log = (min: number, max: number): ParamSpec => ({ min, max, log: true, discrete: false });
const SWITCH: ParamSpec = { min: 0, max: 1, log: false, discrete: true };

export function paramSpec(p: AutomationParam): ParamSpec {
  switch (p.type) {
    case "volume":
    case "sendLevel":
      return linear(-96, 6);
    case "pan":
      return linear(-1, 1);
    case "mute":
      return SWITCH;
    case "insert":
      if (typeof p.param === "object") {
        // Native effect parameter: its range comes from the core's catalogue.
        const d = effectInfo(p.param.effect.kind)?.params[p.param.effect.index];
        if (!d) return SWITCH;
        return { min: d.min, max: d.max, log: d.scale === "log", discrete: d.scale === "choice" || d.scale === "toggle" };
      }
      switch (p.param) {
        case "bypass":
          return SWITCH;
        case "trimGain":
          return linear(-48, 24);
        case "filterCutoff":
          return log(20, 20000);
        case "filterQ":
          return log(0.3, 10);
        case "limiterCeiling":
          return linear(-24, 0);
        case "limiterRelease":
          return log(1, 1000);
      }
  }
}

export function clampValue(spec: ParamSpec, v: number): number {
  const c = Math.min(spec.max, Math.max(spec.min, Number.isFinite(v) ? v : spec.min));
  return spec.discrete ? Math.round(c) : c;
}

export function toNorm(spec: ParamSpec, v: number): number {
  const c = clampValue(spec, v);
  return spec.log ? Math.log2(c / spec.min) / Math.log2(spec.max / spec.min) : (c - spec.min) / (spec.max - spec.min);
}

export function fromNorm(spec: ParamSpec, n: number): number {
  const x = Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0;
  return clampValue(spec, spec.log ? spec.min * Math.pow(spec.max / spec.min, x) : spec.min + x * (spec.max - spec.min));
}

export function bezier(c1: number, c2: number, x: number): number {
  const u = 1 - x;
  return 3 * u * u * x * c1 + 3 * u * x * x * c2 + x * x * x;
}

export function curveShape(curve: Curve, x: number): number {
  const c = Math.min(1, Math.max(0, x));
  if (curve.type === "linear") return c;
  if (curve.type === "step") return 0;
  return bezier(curve.c1, curve.c2, c);
}

interface Piece {
  from: number;
  to: number;
  t0: number;
  t1: number;
  v0: number;
  v1: number;
  curve: Curve;
}

const STEP: Curve = { type: "step" };

/** Mirror of `lane_pieces`: normalised pieces covering [0, ∞). */
export function lanePieces(lane: AutomationLane, base = 0): Piece[] | null {
  const pts = lane.points;
  const first = pts[0];
  if (!first) return null;
  const spec = paramSpec(lane.param);
  const out: Piece[] = [];
  const tFirst = base + first.tick;
  const norm = (v: number) => toNorm(spec, v);
  if (tFirst > 0) out.push({ from: 0, to: tFirst, t0: 0, t1: 0, v0: norm(first.value), v1: norm(first.value), curve: STEP });
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i]!;
    const t0 = base + p.tick;
    const next = pts[i + 1];
    if (!next) {
      out.push({ from: t0, to: Infinity, t0, t1: t0, v0: norm(p.value), v1: norm(p.value), curve: STEP });
      break;
    }
    const t1 = base + next.tick;
    if (t1 <= t0) continue;
    const curve = spec.discrete ? STEP : p.curve;
    const v0 = norm(p.value);
    out.push({ from: t0, to: t1, t0, t1, v0, v1: curve.type === "step" ? v0 : norm(next.value), curve });
  }
  return out;
}

function pieceValue(p: Piece, t: number): number {
  if (p.t1 <= p.t0 || p.v0 === p.v1) return p.v0;
  return p.v0 + (p.v1 - p.v0) * curveShape(p.curve, (t - p.t0) / (p.t1 - p.t0));
}

/** Mirror of `overlay`: replaces `base` by `over` inside [from, to). */
export function overlay(base: Piece[], over: Piece[], from: number, to: number): Piece[] {
  const clip = (p: Piece, a: number, b: number): Piece | null => {
    const f = Math.max(p.from, a);
    const e = Math.min(p.to, b);
    return e > f ? { ...p, from: f, to: e } : null;
  };
  const out: Piece[] = [];
  for (const p of base) {
    const c = clip(p, 0, from);
    if (c) out.push(c);
  }
  for (const p of over) {
    const c = clip(p, from, to);
    if (c) out.push(c);
  }
  for (const p of base) {
    const c = clip(p, to, Infinity);
    if (c) out.push(c);
  }
  return out;
}

export function piecesValue(pieces: Piece[], t: number): number | null {
  let lo = 0;
  let hi = pieces.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (pieces[mid]!.to <= t) lo = mid + 1;
    else hi = mid;
  }
  const p = pieces[lo];
  return p ? pieceValue(p, Math.max(t, p.from)) : null;
}

/** Value (parameter unit) of a track lane at tick `t`. */
export function laneValueAt(lane: AutomationLane, t: number): number | null {
  const pieces = lanePieces(lane);
  if (!pieces) return null;
  const n = piecesValue(pieces, t);
  return n === null ? null : fromNorm(paramSpec(lane.param), n);
}

/** Value arriving at `t` from the left. */
export function laneValueBefore(lane: AutomationLane, t: number): number | null {
  const pieces = lanePieces(lane);
  if (!pieces) return null;
  const i = pieces.findIndex((p) => p.to >= t);
  const p = pieces[i];
  if (!p) return null;
  const at = Math.min(Math.max(t, p.from), Number.isFinite(p.to) ? p.to : t);
  return fromNorm(paramSpec(lane.param), pieceValue(p, at));
}

export function sameParam(a: AutomationParam, b: AutomationParam): boolean {
  return paramKey(a) === paramKey(b);
}

export function paramKey(p: AutomationParam): string {
  switch (p.type) {
    case "sendLevel":
      return `send:${p.aux}`;
    case "insert":
      return `insert:${p.id}:${typeof p.param === "string" ? p.param : `${p.param.effect.kind}:${p.param.effect.index}`}`;
    default:
      return p.type;
  }
}

export function sameChannel(a: ChannelRef, b: ChannelRef): boolean {
  return a.type === b.type && (a.type === "master" || (b.type !== "master" && a.id === b.id));
}

export function channelAutomation(project: Project, c: ChannelRef): ChannelAutomation | null {
  if (c.type === "track") return project.tracks.find((t) => t.id === c.id)?.automation ?? null;
  if (c.type === "aux") return project.mixer.auxes.find((a) => a.id === c.id)?.automation ?? null;
  return project.mixer.master.automation;
}

function insertsOf(project: Project, c: ChannelRef): InsertSlot[] {
  if (c.type === "track") return project.tracks.find((t) => t.id === c.id)?.strip.inserts ?? [];
  if (c.type === "aux") return project.mixer.auxes.find((a) => a.id === c.id)?.strip.inserts ?? [];
  return project.mixer.master.inserts;
}

const INSERT_PARAMS: { param: Extract<AutomationParam, { type: "insert" }>["param"]; on: string[] }[] = [
  { param: "bypass", on: ["trim", "utility", "filter", "limiter", "effect"] },
  { param: "trimGain", on: ["trim"] },
  { param: "filterCutoff", on: ["filter"] },
  { param: "filterQ", on: ["filter"] },
  { param: "limiterCeiling", on: ["limiter"] },
  { param: "limiterRelease", on: ["limiter"] },
];

/** Mirror of `writable_params`: what a channel can automate. */
export function writableParams(project: Project, c: ChannelRef): AutomationParam[] {
  if (c.type === "track" && project.tracks.find((t) => t.id === c.id)?.kind === "midi") return [];
  const out: AutomationParam[] = [{ type: "volume" }];
  if (c.type !== "master") out.push({ type: "pan" });
  out.push({ type: "mute" });
  const sends = c.type === "track" ? project.tracks.find((t) => t.id === c.id)?.strip.sends : c.type === "aux" ? project.mixer.auxes.find((a) => a.id === c.id)?.strip.sends : [];
  for (const s of sends ?? []) out.push({ type: "sendLevel", aux: s.target });
  for (const slot of insertsOf(project, c)) {
    for (const ip of INSERT_PARAMS) if (ip.on.includes(slot.processor.type)) out.push({ type: "insert", id: slot.id, param: ip.param });
    if (slot.processor.type === "effect") {
      // Native effect parameters, by their append-only index (fixed ones excluded).
      const kind = slot.processor.kind;
      for (const p of effectInfo(kind)?.params ?? []) {
        if (p.automatable) out.push({ type: "insert", id: slot.id, param: { effect: { kind, index: p.index } } });
      }
    }
  }
  return out;
}

/** Static (unautomated) value of a parameter. */
export function staticValue(project: Project, c: ChannelRef, p: AutomationParam): number | null {
  const strip =
    c.type === "track"
      ? project.tracks.find((t) => t.id === c.id)
      : c.type === "aux"
        ? project.mixer.auxes.find((a) => a.id === c.id)
        : null;
  const master = project.mixer.master;
  switch (p.type) {
    case "volume":
      return strip ? strip.volumeDb : master.volumeDb;
    case "pan":
      return strip ? strip.pan : null;
    case "mute":
      return (strip ? strip.muted : master.muted) ? 1 : 0;
    case "sendLevel":
      return strip?.strip.sends.find((s) => s.target === p.aux)?.levelDb ?? null;
    case "insert": {
      const slot = insertsOf(project, c).find((s) => s.id === p.id);
      if (!slot) return null;
      const x = slot.processor;
      if (typeof p.param === "object") {
        const d = effectInfo(p.param.effect.kind)?.params[p.param.effect.index];
        return x.type === "effect" && d ? (x.params[d.key] ?? d.default) : null;
      }
      switch (p.param) {
        case "bypass":
          return slot.bypass ? 1 : 0;
        case "trimGain":
          return x.type === "trim" ? x.gainDb : null;
        case "filterCutoff":
          return x.type === "filter" ? x.cutoffHz : null;
        case "filterQ":
          return x.type === "filter" ? x.q : null;
        case "limiterCeiling":
          return x.type === "limiter" ? x.ceilingDb : null;
        case "limiterRelease":
          return x.type === "limiter" ? x.releaseMs : null;
      }
    }
  }
}

const INSERT_LABEL: Record<string, string> = {
  bypass: "Bypass",
  trimGain: "Trim gain",
  filterCutoff: "Cutoff",
  filterQ: "Q",
  limiterCeiling: "Ceiling",
  limiterRelease: "Release",
};

export function paramLabel(project: Project, c: ChannelRef, p: AutomationParam): string {
  switch (p.type) {
    case "volume":
      return "Volume";
    case "pan":
      return "Pan";
    case "mute":
      return "Mute";
    case "sendLevel":
      return `Send → ${project.mixer.auxes.find((a) => a.id === p.aux)?.name ?? "?"}`;
    case "insert": {
      const inserts = insertsOf(project, c);
      const i = inserts.findIndex((s) => s.id === p.id);
      const x = inserts[i]?.processor;
      if (typeof p.param === "object") {
        const info = effectInfo(p.param.effect.kind);
        return `${i + 1}. ${info?.short ?? p.param.effect.kind} · ${info?.params[p.param.effect.index]?.label ?? `Parámetro ${p.param.effect.index + 1}`}`;
      }
      const kind = x?.type === "effect" ? (effectInfo(x.kind)?.short ?? x.kind) : (x?.type ?? "insert");
      return `${i + 1}. ${kind[0]!.toUpperCase()}${kind.slice(1)} · ${INSERT_LABEL[p.param]}`;
    }
  }
}

export function formatParamValue(p: AutomationParam, v: number): string {
  switch (p.type) {
    case "volume":
    case "sendLevel":
      return `${formatDb(v)} dB`;
    case "pan":
      return formatPan(v);
    case "mute":
      return v >= 0.5 ? "Muted" : "On";
    case "insert":
      if (typeof p.param === "object") {
        const d = effectInfo(p.param.effect.kind)?.params[p.param.effect.index];
        return d ? formatParam(d, v) : v.toFixed(2);
      }
      switch (p.param) {
        case "bypass":
          return v >= 0.5 ? "Bypassed" : "Active";
        case "filterCutoff":
          return v >= 1000 ? `${(v / 1000).toFixed(2)} kHz` : `${Math.round(v)} Hz`;
        case "filterQ":
          return v.toFixed(2);
        case "limiterRelease":
          return `${Math.round(v)} ms`;
        default:
          return `${v.toFixed(1)} dB`;
      }
  }
}

export const MODES: { mode: AutomationMode; label: string; title: string }[] = [
  { mode: "off", label: "Off", title: "Automation is kept but ignored" },
  { mode: "read", label: "Read", title: "Play the automation; controls never write" },
  { mode: "touch", label: "Touch", title: "Write while a control is held, then return to the lane" },
  { mode: "latch", label: "Latch", title: "Write from the first touch until playback stops" },
  { mode: "write", label: "Write", title: "Overwrite every automated parameter while playing" },
];

export function writesMode(mode: AutomationMode | undefined): boolean {
  return mode === "touch" || mode === "latch" || mode === "write";
}

/** Value at `tick` of a channel parameter as the engine plays it (track
 * lane with region lanes overlaid), or `null` when it is not automated. */
export function automatedValue(project: Project, c: ChannelRef, p: AutomationParam, tick: number): number | null {
  const auto = channelAutomation(project, c);
  if (!auto || auto.mode === "off") return null;
  const lane = auto.lanes.find((l) => sameParam(l.param, p) && l.points.length > 0);
  const regions: Region[] = c.type === "track" ? (project.tracks.find((t) => t.id === c.id)?.regions ?? []) : [];
  const inside = regions
    .filter((r) => !r.muted && tick >= r.start && tick < r.start + r.length)
    .filter((r) => r.automation.some((l) => sameParam(l.param, p) && l.points.length > 0))
    .sort((a, b) => a.start - b.start)
    .pop();
  if (inside) {
    const rl = inside.automation.find((l) => sameParam(l.param, p))!;
    const pieces = lanePieces(rl, inside.start);
    const n = pieces ? piecesValue(pieces, tick) : null;
    return n === null ? null : fromNorm(paramSpec(p), n);
  }
  return lane ? laneValueAt(lane, tick) : null;
}
