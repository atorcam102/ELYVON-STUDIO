import { memo } from "react";
import { gridFor, ticksToPx } from "../../lib/geometry";
import { gridBackground, type GridLayer } from "../../lib/gridBackground";
import type { TempoMap } from "../../lib/tempoMap";

interface MeterGridProps {
  map: TempoMap;
  pxPerQuarter: number;
  /** Total width in px; the last segment stops here. */
  width: number;
  /** Builds the tiled layers for one metre segment. */
  layers: (g: ReturnType<typeof gridFor>) => GridLayer[];
  className: string;
}

/**
 * Bar/beat grid that follows metre changes: one tiled background per metre
 * segment, each starting on its first bar line (no DOM node per line).
 */
export const MeterGrid = memo(function MeterGrid({ map, pxPerQuarter, width, layers, className }: MeterGridProps) {
  return (
    <>
      {map.segments(0, Infinity).map((seg) => {
        const left = ticksToPx(seg.start, pxPerQuarter);
        if (left >= width) return null;
        const right = Number.isFinite(seg.end) ? Math.min(width, ticksToPx(seg.end, pxPerQuarter)) : width;
        return (
          <div
            key={seg.start}
            className={className}
            style={{ left, width: right - left, ...gridBackground(layers(gridFor(seg.ts, pxPerQuarter))) }}
          />
        );
      })}
    </>
  );
});
