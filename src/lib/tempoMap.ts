// Presentation mirror of `elyvon_core::tempo::TempoMap` and
// `elyvon_core::grid::GridSpec`: bar lines with metre changes, clock times
// with tempo changes, and snapping. The engine never uses this — it only lets
// the ruler, grid and drag previews follow the same rules as the native core.
// Kept in sync by the golden vectors in
// `crates/elyvon-core/tests/fixtures/grid_golden.json` (see tempoMap.test.ts).

import type { GridSpec } from "../bindings/GridSpec";
import type { MeterChange } from "../bindings/MeterChange";
import type { TempoChange } from "../bindings/TempoChange";
import type { TimeSignature } from "../bindings/TimeSignature";
import type { Transport } from "../bindings/Transport";
import { PPQ, ticksPerBar, ticksPerBeat } from "./music";

interface TempoSeg {
  tick: number;
  bpm: number;
  seconds: number;
}

interface MeterSeg {
  bar: number;
  tick: number;
  ts: TimeSignature;
}

export interface BarInfo {
  /** Zero-based bar index. */
  bar: number;
  start: number;
  ts: TimeSignature;
}

function upperBound<T>(items: T[], pred: (item: T) => boolean): number {
  let lo = 0;
  let hi = items.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (pred(items[mid]!)) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

export class TempoMap {
  private readonly tempo: TempoSeg[];
  private readonly meters: MeterSeg[];

  constructor(bpm: number, tempoChanges: TempoChange[], ts: TimeSignature, meterChanges: MeterChange[]) {
    this.tempo = [{ tick: 0, bpm, seconds: 0 }];
    for (const c of tempoChanges) {
      const prev = this.tempo[this.tempo.length - 1]!;
      this.tempo.push({ tick: c.tick, bpm: c.bpm, seconds: prev.seconds + ((c.tick - prev.tick) / PPQ) * (60 / prev.bpm) });
    }
    this.meters = [{ bar: 0, tick: 0, ts }];
    for (const m of meterChanges) {
      const prev = this.meters[this.meters.length - 1]!;
      this.meters.push({ bar: m.bar, tick: prev.tick + (m.bar - prev.bar) * ticksPerBar(prev.ts), ts: m.timeSignature });
    }
  }

  static fromTransport(t: Transport): TempoMap {
    return new TempoMap(t.tempo, t.tempoChanges, t.timeSignature, t.meterChanges);
  }

  bpmAt(tick: number): number {
    return this.tempo[Math.max(0, upperBound(this.tempo, (s) => s.tick <= tick) - 1)]!.bpm;
  }

  secondsAt(tick: number): number {
    const t = Math.max(0, tick);
    const seg = this.tempo[Math.max(0, upperBound(this.tempo, (s) => s.tick <= t) - 1)]!;
    return seg.seconds + ((t - seg.tick) / PPQ) * (60 / seg.bpm);
  }

  tickAtSeconds(seconds: number): number {
    const s = Math.max(0, seconds);
    const seg = this.tempo[Math.max(0, upperBound(this.tempo, (x) => x.seconds <= s) - 1)]!;
    return seg.tick + (s - seg.seconds) * (seg.bpm / 60) * PPQ;
  }

  timeSignatureAt(tick: number): TimeSignature {
    return this.meters[Math.max(0, upperBound(this.meters, (m) => m.tick <= tick) - 1)]!.ts;
  }

  barStart(bar: number): number {
    const m = this.meters[Math.max(0, upperBound(this.meters, (x) => x.bar <= bar) - 1)]!;
    return m.tick + (bar - m.bar) * ticksPerBar(m.ts);
  }

  barAt(tick: number): BarInfo {
    const t = Math.max(0, Math.floor(tick));
    const m = this.meters[Math.max(0, upperBound(this.meters, (x) => x.tick <= t) - 1)]!;
    const per = ticksPerBar(m.ts);
    const n = Math.floor((t - m.tick) / per);
    return { bar: m.bar + n, start: m.tick + n * per, ts: m.ts };
  }

  /** 1-based "bar.beat.tick" as in `MusicalPosition`'s Display. */
  position(tick: number): { bar: number; beat: number; tick: number } {
    const b = this.barAt(tick);
    const perBeat = ticksPerBeat(b.ts);
    const inBar = Math.floor(tick) - b.start;
    return { bar: b.bar + 1, beat: Math.floor(inBar / perBeat) + 1, tick: inBar % perBeat };
  }

  formatPosition(tick: number): string {
    const p = this.position(tick);
    return `${p.bar}.${p.beat}.${String(p.tick).padStart(3, "0")}`;
  }

  /** Metre segments overlapping [from, to): for drawing grids. */
  segments(from: number, to: number): { start: number; end: number; bar: number; ts: TimeSignature }[] {
    const out = [];
    for (let i = 0; i < this.meters.length; i++) {
      const m = this.meters[i]!;
      const end = this.meters[i + 1]?.tick ?? Infinity;
      if (end <= from || m.tick >= to) continue;
      out.push({ start: m.tick, end, bar: m.bar, ts: m.ts });
    }
    return out;
  }

  beats(from: number, to: number): { tick: number; downbeat: boolean }[] {
    const out = [];
    let bar = this.barAt(from);
    let per = ticksPerBeat(bar.ts);
    let next = bar.start + Math.ceil((from - bar.start) / per) * per;
    for (;;) {
      const barEnd = bar.start + ticksPerBar(bar.ts);
      if (next >= barEnd) {
        bar = this.barAt(barEnd);
        per = ticksPerBeat(bar.ts);
        next = bar.start;
      }
      if (next >= to) break;
      out.push({ tick: next, downbeat: next === bar.start });
      next += per;
    }
    return out;
  }
}

export function gridStep(spec: GridSpec, ts: TimeSignature): number | null {
  let base: number;
  switch (spec.division) {
    case "bar":
      return null;
    case "beat":
      base = ticksPerBeat(ts);
      break;
    case "half":
      base = PPQ * 2;
      break;
    case "quarter":
      base = PPQ;
      break;
    case "eighth":
      base = PPQ / 2;
      break;
    case "sixteenth":
      base = PPQ / 4;
      break;
    case "thirtySecond":
      base = PPQ / 8;
      break;
  }
  return spec.triplet ? Math.floor((base * 2) / 3) : base;
}

function swingOffset(spec: GridSpec, step: number, index: number): number {
  if (spec.triplet || spec.swing === 0 || index % 2 === 0) return 0;
  return Math.floor((step * Math.min(spec.swing, 100)) / 300);
}

/** Nearest grid point (same rule as `GridSpec::snap`). */
export function snapTick(map: TempoMap, tick: number, spec: GridSpec): number {
  const t = Math.max(0, Math.round(tick));
  const bar = map.barAt(t);
  const end = bar.start + ticksPerBar(bar.ts);
  const step = gridStep(spec, bar.ts);
  if (step === null) return t - bar.start <= end - t ? bar.start : end;
  const k = Math.floor((t - bar.start) / step);
  let best = end;
  for (let i = Math.max(0, k - 1); i <= k + 1; i++) {
    const p = Math.min(bar.start + i * step + swingOffset(spec, step, i), end);
    if (Math.abs(p - t) < Math.abs(best - t)) best = p;
  }
  return best;
}

/** Grid points in [from, to), bar lines included. */
export function gridPoints(map: TempoMap, from: number, to: number, spec: GridSpec): number[] {
  const out: number[] = [];
  let bar = map.barAt(from);
  while (bar.start < to) {
    const end = bar.start + ticksPerBar(bar.ts);
    const step = gridStep(spec, bar.ts);
    if (step === null) {
      if (bar.start >= from) out.push(bar.start);
    } else {
      for (let i = 0; ; i++) {
        const p = bar.start + i * step + swingOffset(spec, step, i);
        if (p >= end || p >= to) break;
        if (p >= from) out.push(p);
      }
    }
    bar = map.barAt(end);
  }
  return out;
}

export function formatClockSeconds(seconds: number): string {
  const s = Math.max(0, seconds);
  const m = Math.floor(s / 60);
  return `${String(m).padStart(2, "0")}:${(s - m * 60).toFixed(3).padStart(6, "0")}`;
}

export const GRID_LABELS: Record<GridSpec["division"], string> = {
  bar: "Bar",
  beat: "Beat",
  half: "1/2",
  quarter: "1/4",
  eighth: "1/8",
  sixteenth: "1/16",
  thirtySecond: "1/32",
};
