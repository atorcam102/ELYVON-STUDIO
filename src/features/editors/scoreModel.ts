// Layout helpers for the simplified score view (pure, unit-tested).
import type { Region } from "../../bindings/Region";
import type { TempoMap } from "../../lib/tempoMap";

/** Canvas x where the first bar starts; matches `.score-ruler`'s left padding. */
export const SCORE_LEFT = 153;

const LETTER = [0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5, 6];
const SHARP = [false, true, false, true, false, false, true, false, true, false, true, false];

/** Diatonic staff step (C-1 = 0, C4 = 35) and whether the pitch is spelt with a sharp. */
export function staffStep(pitch: number): { step: number; sharp: boolean } {
  const pc = ((pitch % 12) + 12) % 12;
  return { step: Math.floor(pitch / 12) * 7 + LETTER[pc]!, sharp: SHARP[pc]! };
}

/** Bars of `region` (or the first bars of the song) laid out at a zoom of 0–100. */
export function scoreLayout(map: TempoMap, region: Region | null, width: number, zoom: number) {
  const target = 60 + zoom * 3;
  const visible = Math.max(1, Math.round(Math.max(0, width - SCORE_LEFT) / target));
  const barWidth = Math.max(40, (width - SCORE_LEFT) / visible);
  if (!region) return { firstBar: 0, totalBars: visible, visible, barWidth };
  const firstBar = map.barAt(region.start).bar;
  const lastBar = map.barAt(Math.max(region.start, region.start + region.length - 1)).bar;
  return { firstBar, totalBars: lastBar - firstBar + 1, visible, barWidth };
}
