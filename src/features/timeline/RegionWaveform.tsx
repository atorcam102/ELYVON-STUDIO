import { memo, useEffect, useRef, useState } from "react";
import { decodePeaks, inTauri, ipc, type Peaks } from "../../lib/ipc";

interface RegionWaveformProps {
  regionId: string;
  /** Changes whenever anything that affects the drawing changes. */
  revision: string;
  /** Visible tick range inside the region, and its width in CSS px. */
  fromTick: number;
  toTick: number;
  widthPx: number;
  /** Offset of the canvas from the region's left edge, in px. */
  leftPx: number;
}

// Small LRU of decoded responses: scrolling back and forth or toggling
// selection re-draws without asking the core again.
const cache = new Map<string, Peaks>();
const CACHE_LIMIT = 400;

function remember(key: string, peaks: Peaks) {
  cache.delete(key);
  cache.set(key, peaks);
  if (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value!);
}

function draw(canvas: HTMLCanvasElement, peaks: Peaks) {
  const { width, height } = canvas.getBoundingClientRect();
  if (!width || !height) return;
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.ceil(width * ratio);
  canvas.height = Math.ceil(height * ratio);
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.scale(ratio, ratio);
  ctx.fillStyle = "rgba(255,255,255,0.85)";
  const stereo = peaks.data.length >= peaks.columns * 4 && !sameChannels(peaks);
  const lanes = stereo ? 2 : 1;
  const laneH = height / lanes;
  const colW = width / Math.max(1, peaks.columns);
  for (let lane = 0; lane < lanes; lane++) {
    const mid = laneH * lane + laneH / 2;
    const scale = laneH / 2 - 0.5;
    for (let c = 0; c < peaks.columns; c++) {
      const lo = peaks.data[c * 4 + lane * 2]!;
      const hi = peaks.data[c * 4 + lane * 2 + 1]!;
      const top = mid - hi * scale;
      const h = Math.max(1, (hi - lo) * scale);
      ctx.fillRect(c * colW, top, Math.max(1, colW), h);
    }
  }
}

function sameChannels(p: Peaks): boolean {
  for (let c = 0; c < p.columns; c++) {
    if (p.data[c * 4] !== p.data[c * 4 + 2] || p.data[c * 4 + 1] !== p.data[c * 4 + 3]) return false;
  }
  return true;
}

/**
 * Waveform of the visible part of an audio region, drawn from min/max peak
 * columns computed by the native core from the decoded file (through the
 * tempo map, offset and looping). React never touches sample data.
 */
export const RegionWaveform = memo(function RegionWaveform({ regionId, revision, fromTick, toTick, widthPx, leftPx }: RegionWaveformProps) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<"loading" | "ready" | "unavailable">("loading");
  const columns = Math.max(1, Math.min(8192, Math.round(widthPx)));
  // Quantise the request so tiny scrolls reuse cached columns.
  const key = `${regionId}|${revision}|${Math.round(fromTick)}|${Math.round(toTick)}|${columns}`;

  useEffect(() => {
    const el = canvas.current;
    if (!el || !inTauri || toTick <= fromTick) return;
    const hit = cache.get(key);
    if (hit) {
      draw(el, hit);
      setState("ready");
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      ipc.media
        .regionPeaks(regionId, fromTick, toTick, columns)
        .then((buffer) => {
          if (cancelled) return;
          const peaks = decodePeaks(buffer);
          remember(key, peaks);
          draw(el, peaks);
          setState("ready");
        })
        .catch(() => !cancelled && setState("unavailable"));
    }, 40);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [key, regionId, fromTick, toTick, columns]);

  return (
    <canvas
      ref={canvas}
      className={`region__waveform${state === "ready" ? "" : " is-pending"}`}
      style={{ left: leftPx, width: widthPx }}
      data-testid="waveform"
      data-state={state}
      aria-hidden
    />
  );
});
