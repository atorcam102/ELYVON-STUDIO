// Presentation model of the piano roll: geometry, hit testing, selection and
// the edit commands a gesture produces. Notes are edited only by sending
// these commands to the native core; nothing here mutates the project.

import type { EditCommand } from "../../bindings/EditCommand";
import type { Key } from "../../bindings/Key";
import type { MidiNote } from "../../bindings/MidiNote";
import type { Project } from "../../bindings/Project";
import type { StoredMidiEvent } from "../../bindings/StoredMidiEvent";

export interface RollRegion {
  id: string;
  trackId: string;
  start: number;
  length: number;
  color: string;
  events: StoredMidiEvent[];
}

export interface RollNote {
  /** `${regionId}:${noteId}` — stable across edits. */
  key: string;
  region: RollRegion;
  note: MidiNote;
  /** Absolute start/end ticks. */
  abs: number;
  end: number;
}

export interface Roll {
  regions: RollRegion[];
  /** Sorted by absolute start. */
  notes: RollNote[];
  maxLength: number;
}

export function noteKey(regionId: string, id: number): string {
  return `${regionId}:${id}`;
}

/** The selected MIDI regions (in project order) and their notes. */
export function collectRoll(project: Project | null | undefined, regionIds: readonly string[]): Roll {
  const regions: RollRegion[] = [];
  const notes: RollNote[] = [];
  let maxLength = 0;
  if (!project) return { regions, notes, maxLength };
  const wanted = new Set(regionIds);
  for (const t of project.tracks) {
    for (const r of t.regions) {
      if (!wanted.has(r.id) || r.content.type !== "midi") continue;
      const region: RollRegion = { id: r.id, trackId: t.id, start: r.start, length: r.length, color: t.color, events: r.midiEvents };
      regions.push(region);
      for (const n of r.content.notes) {
        const abs = r.start + n.start;
        notes.push({ key: noteKey(r.id, n.id), region, note: n, abs, end: abs + n.length });
        if (n.length > maxLength) maxLength = n.length;
      }
    }
  }
  notes.sort((a, b) => a.abs - b.abs || a.note.pitch - b.note.pitch);
  return { regions, notes, maxLength };
}

export interface View {
  pxPerTick: number;
  keyHeight: number;
  scrollX: number;
  scrollY: number;
  width: number;
  height: number;
}

export const tickToX = (v: View, tick: number) => tick * v.pxPerTick - v.scrollX;
export const xToTick = (v: View, x: number) => Math.max(0, (x + v.scrollX) / v.pxPerTick);
export const pitchToY = (v: View, pitch: number) => (127 - pitch) * v.keyHeight - v.scrollY;
export const yToPitch = (v: View, y: number) => Math.max(0, Math.min(127, 127 - Math.floor((y + v.scrollY) / v.keyHeight)));

/** Index range of notes that may intersect `[from, to)` ticks. */
export function visibleRange(roll: Roll, from: number, to: number): [number, number] {
  const lo = lowerBound(roll.notes, from - roll.maxLength);
  const hi = lowerBound(roll.notes, to);
  return [lo, hi];
}

function lowerBound(notes: RollNote[], tick: number): number {
  let lo = 0;
  let hi = notes.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (notes[mid]!.abs < tick) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

export type HitZone = "body" | "start" | "end";

/** Topmost note under a point; edges are 5 px (or a third of short notes). */
export function hitTest(roll: Roll, v: View, x: number, y: number): { note: RollNote; zone: HitZone } | null {
  const tick = xToTick(v, x);
  const pitch = yToPitch(v, y);
  const [lo, hi] = visibleRange(roll, tick, tick + 1);
  for (let i = hi - 1; i >= lo; i--) {
    const n = roll.notes[i]!;
    if (n.note.pitch !== pitch || tick < n.abs || tick >= n.end) continue;
    const x0 = tickToX(v, n.abs);
    const x1 = tickToX(v, n.end);
    const edge = Math.min(5, (x1 - x0) / 3);
    const zone: HitZone = x >= x1 - edge ? "end" : x <= x0 + edge && x1 - x0 > 12 ? "start" : "body";
    return { note: n, zone };
  }
  return null;
}

/** Notes intersecting a tick × pitch rectangle (rubber-band selection). */
export function notesInRect(roll: Roll, t0: number, t1: number, p0: number, p1: number): RollNote[] {
  const [a, b] = [Math.min(t0, t1), Math.max(t0, t1)];
  const [lo, hi] = [Math.min(p0, p1), Math.max(p0, p1)];
  const [i0, i1] = visibleRange(roll, a, b);
  const out: RollNote[] = [];
  for (let i = i0; i < i1; i++) {
    const n = roll.notes[i]!;
    if (n.end > a && n.abs < b && n.note.pitch >= lo && n.note.pitch <= hi) out.push(n);
  }
  return out;
}

/** Selected notes grouped by region (ids within each region). */
export function byRegion(notes: RollNote[]): Map<string, number[]> {
  const out = new Map<string, number[]>();
  for (const n of notes) {
    const ids = out.get(n.region.id) ?? [];
    ids.push(n.note.id);
    out.set(n.region.id, ids);
  }
  return out;
}

/** One undo step for several edits. */
export function single(commands: EditCommand[]): EditCommand | null {
  if (commands.length === 0) return null;
  return commands.length === 1 ? commands[0]! : { type: "batch", commands };
}

export function moveCommand(selected: RollNote[], deltaTicks: number, deltaPitch: number, copy: boolean): EditCommand | null {
  return single(
    [...byRegion(selected)].map(([regionId, ids]) => ({ type: "moveNotes", regionId, ids, deltaTicks, deltaPitch, copy })),
  );
}

export function resizeCommand(selected: RollNote[], edge: "start" | "end", delta: number): EditCommand | null {
  return single([...byRegion(selected)].map(([regionId, ids]) => ({ type: "resizeNotes", regionId, ids, edge, delta })));
}

export function removeCommand(selected: RollNote[]): EditCommand | null {
  return single([...byRegion(selected)].map(([regionId, ids]) => ({ type: "removeNotes", regionId, ids })));
}

/** Replaces fields of the given notes (velocity lane, inspector). */
export function updateCommand(changed: RollNote[], patch: (n: MidiNote, from: RollNote) => MidiNote): EditCommand | null {
  const map = new Map<string, MidiNote[]>();
  for (const n of changed) {
    const list = map.get(n.region.id) ?? [];
    list.push(patch(n.note, n));
    map.set(n.region.id, list);
  }
  return single([...map].map(([regionId, notes]) => ({ type: "updateNotes", regionId, notes })));
}

/** Keys of notes created by a (batched) copy, in the order of `selected`. */
export function createdKeys(selected: RollNote[], created: number[]): string[] {
  const keys: string[] = [];
  let i = 0;
  for (const [regionId, ids] of byRegion(selected)) {
    for (let k = 0; k < ids.length && i < created.length; k++) keys.push(noteKey(regionId, created[i++]!));
  }
  return keys;
}

const MAJOR = [0, 2, 4, 5, 7, 9, 11];
const MINOR = [0, 2, 3, 5, 7, 8, 10];
const PITCH_INDEX: Record<Key["tonic"], number> = {
  C: 0,
  "C#": 1,
  D: 2,
  "D#": 3,
  E: 4,
  F: 5,
  "F#": 6,
  G: 7,
  "G#": 8,
  A: 9,
  "A#": 10,
  B: 11,
};

/** Whether `pitch` belongs to the project key (scale highlighting). */
export function inScale(pitch: number, key: Key): boolean {
  const tonic = PITCH_INDEX[key.tonic] ?? 0;
  const steps = key.mode === "minor" ? MINOR : MAJOR;
  return steps.includes((((pitch - tonic) % 12) + 12) % 12);
}

export function isBlackKey(pitch: number): boolean {
  return [1, 3, 6, 8, 10].includes(pitch % 12);
}

/** Controller lanes offered under the notes. */
export type LaneKind = "velocity" | "cc1" | "cc7" | "cc11" | "cc64" | "pitchBend" | "pressure" | "program";

export const LANES: { kind: LaneKind; label: string; max: number }[] = [
  { kind: "velocity", label: "Velocity", max: 127 },
  { kind: "cc1", label: "Modulation (CC1)", max: 127 },
  { kind: "cc7", label: "Volume (CC7)", max: 127 },
  { kind: "cc11", label: "Expression (CC11)", max: 127 },
  { kind: "cc64", label: "Sustain (CC64)", max: 127 },
  { kind: "pitchBend", label: "Pitch Bend", max: 16383 },
  { kind: "pressure", label: "Channel Pressure", max: 127 },
  { kind: "program", label: "Program Change", max: 127 },
];

/** The native lane descriptor for a lane kind and channel (0-based). */
export function laneOf(kind: Exclude<LaneKind, "velocity">, channel: number) {
  switch (kind) {
    case "cc1":
      return { type: "controller" as const, channel, controller: 1 };
    case "cc7":
      return { type: "controller" as const, channel, controller: 7 };
    case "cc11":
      return { type: "controller" as const, channel, controller: 11 };
    case "cc64":
      return { type: "controller" as const, channel, controller: 64 };
    case "pitchBend":
      return { type: "pitchBend" as const, channel };
    case "pressure":
      return { type: "channelPressure" as const, channel };
    case "program":
      return { type: "program" as const, channel };
  }
}

/** Value of a stored event in a lane, or `null` if it belongs elsewhere. */
export function laneValue(kind: Exclude<LaneKind, "velocity">, channel: number, bytes: number[]): number | null {
  const s = bytes[0] ?? 0;
  if (s >= 0xf0 || (s & 0x0f) !== channel) return null;
  const hi = s & 0xf0;
  switch (kind) {
    case "pitchBend":
      return hi === 0xe0 ? (bytes[1] ?? 0) | ((bytes[2] ?? 0) << 7) : null;
    case "pressure":
      return hi === 0xd0 ? (bytes[1] ?? 0) : null;
    case "program":
      return hi === 0xc0 ? (bytes[1] ?? 0) : null;
    default: {
      const cc = { cc1: 1, cc7: 7, cc11: 11, cc64: 64 }[kind];
      return hi === 0xb0 && bytes[1] === cc ? (bytes[2] ?? 0) : null;
    }
  }
}
