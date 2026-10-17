// Timeline geometry: conversions between ticks and pixels, grid choice and
// snapping. Pure functions so they can be unit-tested.

import type { TimeSignature } from "../bindings/TimeSignature";
import { PPQ, ticksPerBar, ticksPerBeat } from "./music";

export const MIN_PX_PER_QUARTER = 3;
export const MAX_PX_PER_QUARTER = 320;
export const DEFAULT_PX_PER_QUARTER = 15;
export const TRACK_HEIGHT = 62;
export const RULER_HEIGHT = 38;

export function ticksToPx(ticks: number, pxPerQuarter: number): number {
  return (ticks / PPQ) * pxPerQuarter;
}

export function pxToTicks(px: number, pxPerQuarter: number): number {
  return (px / pxPerQuarter) * PPQ;
}

export function clampZoom(pxPerQuarter: number): number {
  return Math.min(MAX_PX_PER_QUARTER, Math.max(MIN_PX_PER_QUARTER, pxPerQuarter));
}

/** Zooms by `factor` keeping the tick under `anchorPx` (viewport x) fixed. */
export function zoomAround(
  pxPerQuarter: number,
  factor: number,
  scrollLeft: number,
  anchorPx: number,
): { pxPerQuarter: number; scrollLeft: number } {
  const next = clampZoom(pxPerQuarter * factor);
  const anchorTicks = pxToTicks(scrollLeft + anchorPx, pxPerQuarter);
  return { pxPerQuarter: next, scrollLeft: Math.max(0, ticksToPx(anchorTicks, next) - anchorPx) };
}

export interface Grid {
  /** Pixel spacing of bar lines. */
  barPx: number;
  /** Beat line spacing, or 0 when beats are too dense to draw. */
  beatPx: number;
  /** Subdivision spacing, or 0 when hidden. */
  subPx: number;
  /** Snap resolution in ticks (the finest visible line, at least a bar). */
  snapTicks: number;
  /** Draw a bar number every `labelEvery` bars. */
  labelEvery: number;
}

const MIN_LINE_GAP = 7;
const MIN_LABEL_GAP = 44;

export function gridFor(ts: TimeSignature, pxPerQuarter: number): Grid {
  const barTicks = ticksPerBar(ts);
  const beatTicks = ticksPerBeat(ts);
  const barPx = ticksToPx(barTicks, pxPerQuarter);
  const beatPx = ticksToPx(beatTicks, pxPerQuarter);
  let subTicks = 0;
  for (const div of [PPQ / 4, PPQ / 2]) {
    if (div < beatTicks && ticksToPx(div, pxPerQuarter) >= MIN_LINE_GAP * 2) {
      subTicks = div;
      break;
    }
  }
  const showBeats = beatPx >= MIN_LINE_GAP;
  const snapTicks = subTicks || (showBeats ? beatTicks : barTicks);
  let labelEvery = 1;
  while (barPx * labelEvery < MIN_LABEL_GAP) labelEvery *= 2;
  return {
    barPx,
    beatPx: showBeats ? beatPx : 0,
    subPx: subTicks ? ticksToPx(subTicks, pxPerQuarter) : 0,
    snapTicks,
    labelEvery,
  };
}

export function snap(ticks: number, snapTicks: number): number {
  if (snapTicks <= 0) return Math.round(ticks);
  return Math.round(ticks / snapTicks) * snapTicks;
}

/** Half-open visible tick range for a horizontal viewport. */
export function visibleTicks(scrollLeft: number, width: number, pxPerQuarter: number, marginPx = 200) {
  return {
    start: Math.max(0, pxToTicks(scrollLeft - marginPx, pxPerQuarter)),
    end: pxToTicks(scrollLeft + width + marginPx, pxPerQuarter),
  };
}
