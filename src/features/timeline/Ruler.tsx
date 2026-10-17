import { useRef, useState, type PointerEvent } from "react";
import type { Marker } from "../../bindings/Marker";
import type { Transport } from "../../bindings/Transport";
import { Menu } from "../../components/Popover";
import { pxToTicks, ticksToPx } from "../../lib/geometry";
import { ticksPerBar } from "../../lib/music";
import type { TempoMap } from "../../lib/tempoMap";
import { useProject } from "../../state/project";
import { locate } from "../transport/transportActions";
import { MeterGrid } from "./MeterGrid";

interface RulerProps {
  transport: Transport;
  markers: Marker[];
  map: TempoMap;
  width: number;
  pxPerQuarter: number;
  viewLeft: number;
  viewWidth: number;
  cursorTicks: number;
  /** Snaps a tick to the current grid. */
  snap: (ticks: number) => number;
}

const LOOP_STRIP = 13;
const MIN_LABEL_GAP = 90;

/**
 * Bar ruler following metre changes. The upper part locates the playhead
 * (click or drag); the lower strip sets the cycle range (drag) or toggles it
 * (click inside it). Markers sit on the label row; punch shows in red.
 */
export function Ruler({ transport, markers, map, width, pxPerQuarter, viewLeft, viewWidth, cursorTicks, snap }: RulerProps) {
  const apply = useProject((s) => s.apply);
  const gesture = useRef<{ kind: "cursor" | "loop"; startTicks: number } | null>(null);
  const [loopDraft, setLoopDraft] = useState<{ start: number; end: number } | null>(null);
  const [markerMenu, setMarkerMenu] = useState<{ marker: Marker; at: { x: number; y: number } } | null>(null);

  const ticksAt = (e: PointerEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    return Math.max(0, pxToTicks(e.clientX - rect.left, pxPerQuarter));
  };
  const barStartAt = (t: number) => map.barAt(t).start;
  const barEndAt = (t: number) => {
    const b = map.barAt(t);
    return b.start + ticksPerBar(b.ts);
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const rect = e.currentTarget.getBoundingClientRect();
    const t = ticksAt(e);
    if (rect.bottom - e.clientY < LOOP_STRIP) {
      gesture.current = { kind: "loop", startTicks: barStartAt(t) };
    } else {
      gesture.current = { kind: "cursor", startTicks: t };
      locate(e.altKey ? t : snap(t));
    }
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    if (!g) return;
    const t = ticksAt(e);
    if (g.kind === "cursor") {
      locate(e.altKey ? t : snap(t));
    } else {
      const bar = barStartAt(t);
      setLoopDraft({ start: Math.min(g.startTicks, bar), end: Math.max(barEndAt(g.startTicks), barEndAt(t)) });
    }
  };

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    gesture.current = null;
    if (!g || g.kind !== "loop") return;
    const lr = transport.loopRange;
    if (loopDraft) {
      void apply({ type: "setLoop", enabled: true, start: loopDraft.start, end: loopDraft.end });
    } else {
      const t = ticksAt(e);
      if (t >= lr.start && t < lr.end) {
        void apply({ type: "setLoop", enabled: !lr.enabled, start: lr.start, end: lr.end });
      } else {
        void apply({ type: "setLoop", enabled: true, start: g.startTicks, end: barEndAt(g.startTicks) });
      }
    }
    setLoopDraft(null);
  };

  const loop = loopDraft ?? transport.loopRange;
  const loopEnabled = loopDraft ? true : transport.loopRange.enabled;

  // Label only visible bars, thinning out when bars get narrow.
  const fromTick = pxToTicks(Math.max(0, viewLeft - 50), pxPerQuarter);
  const toTick = pxToTicks(viewLeft + viewWidth + 50, pxPerQuarter);
  const labels: { bar: number; left: number }[] = [];
  let lastLeft = -Infinity;
  for (let b = map.barAt(fromTick); b.start < toTick; ) {
    const left = ticksToPx(b.start, pxPerQuarter);
    if (left >= width) break;
    if (left - lastLeft >= MIN_LABEL_GAP) {
      labels.push({ bar: b.bar + 1, left });
      lastLeft = left;
    }
    b = map.barAt(b.start + ticksPerBar(b.ts));
  }
  const meterMarks = transport.meterChanges.map((m) => ({ left: ticksToPx(map.barStart(m.bar), pxPerQuarter), ts: m.timeSignature }));

  return (
    <div
      className="ruler"
      style={{ width }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => {
        gesture.current = null;
        setLoopDraft(null);
      }}
      title="Click or drag to move the playhead (Alt: no snapping). Drag in the lower strip to set the cycle range."
      data-testid="ruler"
    >
      <div className="ruler__loop-strip">
        <div
          className={`ruler__loop${loopEnabled ? " is-enabled" : ""}`}
          style={{ left: ticksToPx(loop.start, pxPerQuarter), width: ticksToPx(loop.end - loop.start, pxPerQuarter) }}
        />
        {transport.punch.enabled && (
          <div
            className="ruler__punch"
            title="Punch range (recording is not available yet)"
            style={{ left: ticksToPx(transport.punch.start, pxPerQuarter), width: ticksToPx(transport.punch.end - transport.punch.start, pxPerQuarter) }}
          />
        )}
      </div>
      <MeterGrid
        map={map}
        pxPerQuarter={pxPerQuarter}
        width={width}
        className="ruler__ticks"
        layers={(g) => [
          { axis: "x", every: g.barPx, color: "var(--ruler-bar)" },
          { axis: "x", every: g.beatPx, color: "var(--ruler-beat)", length: "6px" },
        ]}
      />
      {labels.map((l) => (
        <span key={l.bar} className="ruler__label" style={{ left: l.left }}>
          {l.bar}
        </span>
      ))}
      {meterMarks.map((m) => (
        <span key={m.left} className="ruler__meter" style={{ left: m.left }} title="Time signature change">
          {`${m.ts.numerator}/${m.ts.denominator}`}
        </span>
      ))}
      {markers.map((m) => (
        <button
          key={m.id}
          type="button"
          className="ruler__marker"
          style={{ left: ticksToPx(m.tick, pxPerQuarter) }}
          title={`${m.name} — click to go here, right-click for options`}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => locate(m.tick)}
          onContextMenu={(e) => {
            e.preventDefault();
            setMarkerMenu({ marker: m, at: { x: e.clientX, y: e.clientY } });
          }}
        >
          {m.name}
        </button>
      ))}
      <div className="ruler__cursor" style={{ left: ticksToPx(cursorTicks, pxPerQuarter) }} aria-hidden />
      {markerMenu && (
        <Menu
          at={markerMenu.at}
          onClose={() => setMarkerMenu(null)}
          entries={[
            { heading: markerMenu.marker.name },
            { id: "go", label: "Go to marker", icon: "marker", onSelect: () => locate(markerMenu.marker.tick) },
            {
              id: "move",
              label: "Move marker to cursor",
              onSelect: () => void apply({ type: "updateMarker", id: markerMenu.marker.id, name: markerMenu.marker.name, tick: cursorTicks }),
            },
            { id: "delete", label: "Delete marker", icon: "trash", onSelect: () => void apply({ type: "removeMarkers", ids: [markerMenu.marker.id] }) },
          ]}
        />
      )}
    </div>
  );
}
