import { useRef, useState, type PointerEvent } from "react";
import type { Transport } from "../../bindings/Transport";
import { pxToTicks, snap, ticksToPx, type Grid } from "../../lib/geometry";
import { gridBackground } from "../../lib/gridBackground";
import { ticksPerBar } from "../../lib/music";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";

interface RulerProps {
  transport: Transport;
  width: number;
  lengthBars: number;
  pxPerQuarter: number;
  grid: Grid;
  viewLeft: number;
  viewWidth: number;
  cursorTicks: number;
}

const LOOP_STRIP = 12;

/**
 * Bar ruler. The lower part places the edit cursor (click or drag); the upper
 * strip sets the loop range (drag) or toggles it (click inside the range).
 */
export function Ruler({ transport, width, lengthBars, pxPerQuarter, grid, viewLeft, viewWidth, cursorTicks }: RulerProps) {
  const setCursor = useUi((s) => s.setCursor);
  const apply = useProject((s) => s.apply);
  const barTicks = ticksPerBar(transport.timeSignature);
  const gesture = useRef<{ kind: "cursor" | "loop"; startTicks: number; moved: boolean } | null>(null);
  const [loopDraft, setLoopDraft] = useState<{ start: number; end: number } | null>(null);

  const ticksAt = (e: PointerEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    return Math.max(0, pxToTicks(e.clientX - rect.left, pxPerQuarter));
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const rect = e.currentTarget.getBoundingClientRect();
    const t = ticksAt(e);
    if (e.clientY - rect.top < LOOP_STRIP) {
      gesture.current = { kind: "loop", startTicks: Math.floor(t / barTicks) * barTicks, moved: false };
    } else {
      gesture.current = { kind: "cursor", startTicks: t, moved: false };
      setCursor(e.altKey ? t : snap(t, grid.snapTicks));
    }
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    if (!g) return;
    const t = ticksAt(e);
    g.moved = true;
    if (g.kind === "cursor") {
      setCursor(e.altKey ? t : snap(t, grid.snapTicks));
    } else {
      const bar = Math.floor(t / barTicks) * barTicks;
      const start = Math.min(g.startTicks, bar);
      const end = Math.max(g.startTicks, bar) + barTicks;
      setLoopDraft({ start, end });
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
        void apply({ type: "setLoop", enabled: true, start: g.startTicks, end: g.startTicks + barTicks });
      }
    }
    setLoopDraft(null);
  };

  const loop = loopDraft ?? transport.loopRange;
  const loopEnabled = loopDraft ? true : transport.loopRange.enabled;

  // Only label bars that are on screen.
  const firstBar = Math.max(0, Math.floor(viewLeft / grid.barPx) - 1);
  const lastBar = Math.min(lengthBars, Math.ceil((viewLeft + viewWidth) / grid.barPx) + 1);
  const labels: number[] = [];
  for (let b = firstBar - (firstBar % grid.labelEvery); b <= lastBar; b += grid.labelEvery) {
    if (b >= 0) labels.push(b);
  }

  const tickStyle = gridBackground([
    { axis: "x", every: grid.barPx, color: "var(--ruler-bar)" },
    { axis: "x", every: grid.beatPx, color: "var(--ruler-beat)", length: "6px" },
  ]);

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
      title="Click or drag to place the cursor. Drag in the top strip to set the loop range."
    >
      <div className="ruler__loop-strip">
        <div
          className={`ruler__loop${loopEnabled ? " is-enabled" : ""}`}
          style={{ left: ticksToPx(loop.start, pxPerQuarter), width: ticksToPx(loop.end - loop.start, pxPerQuarter) }}
        />
      </div>
      <div className="ruler__ticks" style={tickStyle} />
      {labels.map((b) => (
        <span key={b} className="ruler__label" style={{ left: b * grid.barPx }}>
          {b + 1}
        </span>
      ))}
      <div className="ruler__cursor" style={{ left: ticksToPx(cursorTicks, pxPerQuarter) }} aria-hidden />
    </div>
  );
}
