// Presentation helpers for musical time. The authoritative model and its
// validation live in Rust (`elyvon-core::music`); this only formats and lays out.

import type { Key } from "../bindings/Key";
import type { PitchClass } from "../bindings/PitchClass";
import type { TimeSignature } from "../bindings/TimeSignature";

/** Ticks per quarter note — must match `elyvon_core::PPQ`. */
export const PPQ = 960;

export function ticksPerBeat(ts: TimeSignature): number {
  return (PPQ * 4) / ts.denominator;
}

export function ticksPerBar(ts: TimeSignature): number {
  return ticksPerBeat(ts) * ts.numerator;
}

export interface BarPosition {
  bar: number;
  beat: number;
  /** Sixteenth-note division within the beat, 1-based. */
  division: number;
  tick: number;
}

/** 1-based bar.beat.division.tick, as shown in the transport display. */
export function positionFromTicks(ticks: number, ts: TimeSignature): BarPosition {
  const perBar = ticksPerBar(ts);
  const perBeat = ticksPerBeat(ts);
  const perDivision = PPQ / 4;
  const t = Math.max(0, Math.floor(ticks));
  const inBar = t % perBar;
  const inBeat = inBar % perBeat;
  return {
    bar: Math.floor(t / perBar) + 1,
    beat: Math.floor(inBar / perBeat) + 1,
    division: Math.floor(inBeat / perDivision) + 1,
    tick: inBeat % perDivision,
  };
}

export function formatPosition(ticks: number, ts: TimeSignature): string {
  const p = positionFromTicks(ticks, ts);
  return `${p.bar}.${p.beat}.${p.division}.${String(p.tick).padStart(3, "0")}`;
}

/** Bar.beat without sub-beat detail, e.g. "5.3". */
export function formatBarBeat(ticks: number, ts: TimeSignature): string {
  const p = positionFromTicks(ticks, ts);
  return `${p.bar}.${p.beat}`;
}

/** Elapsed time at a constant tempo, formatted mm:ss.mmm. */
export function formatClock(ticks: number, bpm: number): string {
  const seconds = (ticks / PPQ) * (60 / bpm);
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  return `${String(m).padStart(2, "0")}:${s.toFixed(3).padStart(6, "0")}`;
}

/** Human-readable duration in bars and beats, e.g. "2 bars 1 beat". */
export function formatLength(ticks: number, ts: TimeSignature): string {
  const perBar = ticksPerBar(ts);
  const perBeat = ticksPerBeat(ts);
  const bars = Math.floor(ticks / perBar);
  const beats = Math.floor((ticks % perBar) / perBeat);
  const rest = ticks % perBeat;
  const parts: string[] = [];
  if (bars) parts.push(`${bars} ${bars === 1 ? "bar" : "bars"}`);
  if (beats) parts.push(`${beats} ${beats === 1 ? "beat" : "beats"}`);
  if (rest || parts.length === 0) parts.push(`${rest} ticks`);
  return parts.join(" ");
}

export const PITCH_CLASSES: PitchClass[] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export function formatKey(key: Key): string {
  return `${key.tonic} ${key.mode === "major" ? "maj" : "min"}`;
}

const NOTE_NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];

/** MIDI note number to name with octave (C4 = 60). */
export function noteName(pitch: number): string {
  return `${NOTE_NAMES[pitch % 12]}${Math.floor(pitch / 12) - 1}`;
}

export const DENOMINATORS = [1, 2, 4, 8, 16, 32] as const;
