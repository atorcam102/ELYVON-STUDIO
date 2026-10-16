import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { EditCommand } from "../../bindings/EditCommand";
import type { GridSpec } from "../../bindings/GridSpec";
import type { MidiNote } from "../../bindings/MidiNote";
import type { NoteTransform } from "../../bindings/NoteTransform";
import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import { Badge, IconButton } from "../../components/Button";
import { NumberField } from "../../components/Fields";
import { Icon } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { ipc } from "../../lib/ipc";
import { noteName, ticksPerBar } from "../../lib/music";
import { GRID_LABELS, gridPoints, gridStep, snapTick, TempoMap } from "../../lib/tempoMap";
import { notifyError } from "../../state/notifications";
import { useProject } from "../../state/project";
import { useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";
import {
  collectRoll,
  createdKeys,
  hitTest,
  inScale,
  isBlackKey,
  LANES,
  laneOf,
  laneValue,
  moveCommand,
  noteKey,
  notesInRect,
  pitchToY,
  removeCommand,
  resizeCommand,
  tickToX,
  updateCommand,
  visibleRange,
  xToTick,
  yToPitch,
  type LaneKind,
  type Roll,
  type RollNote,
  type RollRegion,
  type View,
} from "./pianoRollModel";

const KEYS_W = 52;
const RULER_H = 22;
const LANE_H = 96;
const DIVISIONS: GridSpec["division"][] = ["bar", "quarter", "eighth", "sixteenth", "thirtySecond"];
type Tool = "pointer" | "pencil" | "eraser";

function selectedRegion(project: Project | undefined, ids: string[], type: "midi" | "audio"): Region | undefined {
  if (!project) return undefined;
  const all = project.tracks.flatMap((t) => t.regions);
  for (let i = ids.length - 1; i >= 0; i--) {
    const r = all.find((x) => x.id === ids[i]);
    if (r?.content.type === type) return r;
  }
  return undefined;
}

/** Piano roll of the selected MIDI regions: draw, drag, resize, copy,
 * erase, quantise and edit velocity/controller lanes. Every gesture is one
 * native edit (one undo step). */
export function PianoRoll() {
  const project = useProject((s) => s.view?.project);
  const regionIds = useUi((s) => s.selection.regionIds);
  const roll = useMemo(() => collectRoll(project, regionIds), [project, regionIds]);
  if (!project || roll.regions.length === 0) {
    return (
      <div className="editor-empty">
        <Icon name="piano" size={22} />
        <p>Select one or more MIDI regions to edit their notes.</p>
        <span>Double-click a MIDI track lane to create a region, then draw notes with the pencil (2).</span>
      </div>
    );
  }
  return <PianoRollEditor project={project} roll={roll} />;
}

interface Drag {
  kind: "move" | "start" | "end" | "rubber" | "pencil" | "erase";
  x: number;
  y: number;
  /** Anchor note for snapping. */
  anchor?: RollNote;
  tick: number;
  pitch: number;
  moved: boolean;
  /** Preview offsets (ticks / semitones) or pencil note. */
  dt: number;
  dp: number;
  copy: boolean;
  region?: RollRegion;
  erased?: Set<string>;
  curX?: number;
  curY?: number;
}

interface LaneDrag {
  kind: "velocity" | "draw";
  notes?: RollNote[];
  base?: Map<string, number>;
  points: { tick: number; value: number }[];
  startValue: number;
  y: number;
  value: number;
  erase: boolean;
}

function PianoRollEditor({ project, roll }: { project: Project; roll: Roll }) {
  const apply = useProject((s) => s.apply);
  const map = useMemo(() => TempoMap.fromTransport(project.transport), [project.transport]);
  const [tool, setTool] = useState<Tool>("pointer");
  const [grid, setGrid] = useState<GridSpec>({ division: "sixteenth", triplet: false, swing: 0 });
  const [snap, setSnap] = useState(true);
  const [strength, setStrength] = useState(100);
  const [scale, setScale] = useState(true);
  const [audition, setAudition] = useState(true);
  const [lane, setLane] = useState<LaneKind>("velocity");
  const [laneChannel, setLaneChannel] = useState(0);
  const [selection, setSelection] = useState<Set<string>>(() => new Set());
  const [ppt, setPpt] = useState(0.12);
  const [keyH, setKeyH] = useState(13);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [scroll, setScroll] = useState({ x: 0, y: 0, w: 600, h: 300 });
  const scrollRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const laneRef = useRef<HTMLCanvasElement>(null);
  const drag = useRef<Drag | null>(null);
  const laneDrag = useRef<LaneDrag | null>(null);
  const lastLength = useRef(0);
  const frame = useRef(0);
  const drawRef = useRef<() => void>(() => undefined);

  const end = useMemo(() => {
    const regionEnd = Math.max(...roll.regions.map((r) => r.start + r.length));
    const barLen = ticksPerBar(project.transport.timeSignature);
    return Math.max(regionEnd + 2 * barLen, map.barStart(project.lengthBars));
  }, [roll.regions, project.lengthBars, project.transport.timeSignature, map]);

  const view: View = useMemo(
    () => ({ pxPerTick: ppt, keyHeight: keyH, scrollX: scroll.x, scrollY: scroll.y, width: scroll.w - KEYS_W, height: scroll.h - RULER_H }),
    [ppt, keyH, scroll],
  );
  const selected = useMemo(() => roll.notes.filter((n) => selection.has(n.key)), [roll, selection]);
  const primary = roll.regions[roll.regions.length - 1]!;
  const primaryTrack = project.tracks.find((t) => t.id === primary.trackId);

  // Initial position: first region at the left, its notes (or C4) centred.
  const placed = useRef<string | null>(null);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    const first = roll.regions[0]!;
    if (!el || placed.current === first.id) return;
    placed.current = first.id;
    const pitches = roll.notes.map((n) => n.note.pitch);
    const centre = pitches.length ? (Math.min(...pitches) + Math.max(...pitches)) / 2 : 60;
    el.scrollLeft = Math.max(0, first.start * ppt - 16);
    el.scrollTop = Math.max(0, (127 - centre) * keyH - el.clientHeight / 2);
  }, [roll, ppt, keyH]);

  const sync = useCallback(() => {
    const el = scrollRef.current;
    if (el) setScroll({ x: el.scrollLeft, y: el.scrollTop, w: el.clientWidth, h: el.clientHeight });
  }, []);
  useLayoutEffect(() => {
    sync();
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(sync);
    ro.observe(el);
    return () => ro.disconnect();
  }, [sync]);

  const requestDraw = useCallback(() => {
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(() => drawRef.current());
  }, []);
  useEffect(() => useTransport.subscribe((s, p) => s.snapshot?.ticks !== p.snapshot?.ticks && requestDraw()), [requestDraw]);

  const snapTo = (tick: number, free: boolean) => (snap && !free ? snapTick(map, tick, grid) : Math.round(tick));
  const floorGrid = (tick: number) => {
    if (!snap) return Math.round(tick);
    const bar = map.barAt(tick);
    const pts = gridPoints(map, bar.start, Math.floor(tick) + 1, grid);
    return pts[pts.length - 1] ?? bar.start;
  };
  const stepAt = (tick: number) => gridStep(grid, map.timeSignatureAt(tick)) ?? ticksPerBar(map.timeSignatureAt(tick));

  const run = async (cmd: EditCommand | null) => (cmd ? apply(cmd) : null);
  const preview = (n: { pitch: number; velocity: number; channel: number }, trackId = primary.trackId) => {
    if (!audition) return;
    ipc.midi.preview(trackId, n.channel, n.pitch, n.velocity, 300).catch(notifyError);
  };

  // ---------------------------------------------------------------- drawing
  const draw = () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const W = scroll.w;
    const H = scroll.h;
    if (canvas.width !== Math.round(W * dpr) || canvas.height !== Math.round(H * dpr)) {
      canvas.width = Math.round(W * dpr);
      canvas.height = Math.round(H * dpr);
      canvas.style.width = `${W}px`;
      canvas.style.height = `${H}px`;
    }
    const g = canvas.getContext("2d");
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    const css = getComputedStyle(canvas);
    const col = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
    const bg = col("--editor-bg", "#2f2f2f");
    g.fillStyle = bg;
    g.fillRect(0, 0, W, H);
    const v = view;
    const gx = KEYS_W;
    const gy = RULER_H;
    const t0 = xToTick(v, 0);
    const t1 = xToTick(v, v.width);
    const pTop = yToPitch(v, 0);
    const pBottom = yToPitch(v, v.height);

    g.save();
    g.beginPath();
    g.rect(gx, gy, W - gx, H - gy);
    g.clip();
    g.translate(gx, gy);
    // Rows.
    for (let p = pBottom; p <= pTop; p++) {
      const y = pitchToY(v, p);
      const out = scale && !inScale(p, project.transport.key);
      g.fillStyle = isBlackKey(p) ? "rgba(0,0,0,0.22)" : out ? "rgba(0,0,0,0.1)" : "rgba(255,255,255,0.02)";
      g.fillRect(0, y, v.width, v.keyHeight);
      if (p % 12 === 0) {
        g.fillStyle = "rgba(255,255,255,0.10)";
        g.fillRect(0, y + v.keyHeight - 1, v.width, 1);
      }
    }
    // Outside the edited regions is dimmed.
    g.fillStyle = "rgba(0,0,0,0.28)";
    g.fillRect(0, 0, v.width, v.height);
    for (const r of roll.regions) {
      const x0 = tickToX(v, r.start);
      const x1 = tickToX(v, r.start + r.length);
      g.fillStyle = "rgba(255,255,255,0.035)";
      g.fillRect(x0, 0, x1 - x0, v.height);
      g.fillStyle = r.color;
      g.globalAlpha = 0.55;
      g.fillRect(x0, 0, 1, v.height);
      g.fillRect(x1 - 1, 0, 1, v.height);
      g.globalAlpha = 1;
    }
    // Grid, beats and bars (metre-aware).
    const stepPx = (gridStep(grid, map.timeSignatureAt(t0)) ?? ticksPerBar(map.timeSignatureAt(t0))) * v.pxPerTick;
    if (stepPx >= 6) {
      g.fillStyle = "rgba(255,255,255,0.05)";
      for (const t of gridPoints(map, Math.floor(t0), Math.ceil(t1) + 1, grid)) g.fillRect(Math.round(tickToX(v, t)), 0, 1, v.height);
    }
    g.fillStyle = "rgba(255,255,255,0.10)";
    for (const b of map.beats(Math.floor(t0), Math.ceil(t1) + 1)) g.fillRect(Math.round(tickToX(v, b.tick)), 0, 1, v.height);
    g.fillStyle = "rgba(255,255,255,0.22)";
    for (const s of map.segments(Math.floor(t0), Math.ceil(t1) + 1)) g.fillRect(Math.round(tickToX(v, s.start)), 0, 1, v.height);

    // Notes.
    const d = drag.current;
    const erased = d?.kind === "erase" ? d.erased : undefined;
    const [i0, i1] = visibleRange(roll, t0, t1);
    const drawNote = (abs: number, len: number, pitch: number, n: MidiNote, color: string, isSel: boolean, ghost: boolean) => {
      const x = tickToX(v, abs);
      const w = Math.max(2, len * v.pxPerTick - 1);
      const y = pitchToY(v, pitch) + 1;
      const h = Math.max(2, v.keyHeight - 2);
      if (x > v.width || x + w < 0 || y > v.height || y + h < 0) return;
      g.globalAlpha = ghost ? 0.35 : n.muted ? 0.35 : 0.55 + (n.velocity / 127) * 0.45;
      g.fillStyle = n.muted ? "#8a8a8a" : color;
      g.fillRect(x, y, w, h);
      g.globalAlpha = 1;
      if (isSel) {
        g.strokeStyle = "#ffffff";
        g.lineWidth = 1.5;
        g.strokeRect(x + 0.75, y + 0.75, w - 1.5, h - 1.5);
      } else {
        g.fillStyle = "rgba(0,0,0,0.25)";
        g.fillRect(x, y + h - 1, w, 1);
      }
      // Velocity bar inside the note.
      g.fillStyle = "rgba(255,255,255,0.45)";
      g.fillRect(x + 1, y + h / 2 - 0.5, Math.max(0, (w - 2) * (n.velocity / 127)), 1);
      if (w > 28 && h >= 10) {
        g.fillStyle = "rgba(0,0,0,0.75)";
        g.font = "9px system-ui, sans-serif";
        g.fillText(noteName(pitch), x + 3, y + h - 3);
      }
    };
    for (let i = i0; i < i1; i++) {
      const n = roll.notes[i]!;
      if (erased?.has(n.key)) continue;
      const isSel = selection.has(n.key);
      if (d && isSel && d.moved && (d.kind === "move" || d.kind === "start" || d.kind === "end")) {
        if (d.kind === "move") {
          if (d.copy) drawNote(n.abs, n.note.length, n.note.pitch, n.note, n.region.color, false, true);
          const p = Math.max(0, Math.min(127, n.note.pitch + d.dp));
          drawNote(Math.max(n.region.start, n.abs + d.dt), n.note.length, p, n.note, n.region.color, true, false);
        } else if (d.kind === "end") {
          drawNote(n.abs, Math.max(1, n.note.length + d.dt), n.note.pitch, n.note, n.region.color, true, false);
        } else {
          const s = Math.min(n.end - 1, Math.max(n.region.start, n.abs + d.dt));
          drawNote(s, n.end - s, n.note.pitch, n.note, n.region.color, true, false);
        }
        continue;
      }
      drawNote(n.abs, n.note.length, n.note.pitch, n.note, n.region.color, isSel, false);
    }
    if (d?.kind === "pencil" && d.region) {
      const len = Math.max(1, d.dt);
      drawNote(d.tick, len, d.pitch, { id: 0, pitch: d.pitch, velocity: 100, channel: 0, muted: false, start: 0, length: len, quantizeOffset: 0 }, d.region.color, true, false);
    }
    if (d?.kind === "rubber" && d.curX !== undefined && d.curY !== undefined) {
      g.strokeStyle = "rgba(255,255,255,0.8)";
      g.fillStyle = "rgba(47,122,229,0.18)";
      const x = Math.min(d.x, d.curX) - gx;
      const y = Math.min(d.y, d.curY) - gy;
      g.fillRect(x, y, Math.abs(d.curX - d.x), Math.abs(d.curY - d.y));
      g.strokeRect(x + 0.5, y + 0.5, Math.abs(d.curX - d.x), Math.abs(d.curY - d.y));
    }
    const head = useTransport.getState().snapshot?.ticks;
    if (head !== undefined) {
      g.fillStyle = "#ffffff";
      g.fillRect(Math.round(tickToX(v, head)), 0, 1, v.height);
    }
    g.restore();

    // Ruler.
    g.fillStyle = col("--ruler", "#333");
    g.fillRect(gx, 0, W - gx, gy);
    g.save();
    g.beginPath();
    g.rect(gx, 0, W - gx, gy);
    g.clip();
    g.font = "10px system-ui, sans-serif";
    for (const s of map.segments(Math.floor(t0), Math.ceil(t1) + 1)) {
      const x = gx + tickToX(v, s.start);
      g.fillStyle = "rgba(255,255,255,0.35)";
      g.fillRect(Math.round(x), 10, 1, gy - 10);
      g.fillStyle = "rgba(255,255,255,0.75)";
      g.fillText(`${s.bar + 1}${s.ts.numerator !== 4 || s.ts.denominator !== 4 ? ` ${s.ts.numerator}/${s.ts.denominator}` : ""}`, x + 3, 11);
    }
    for (const r of roll.regions) {
      g.fillStyle = r.color;
      g.globalAlpha = 0.7;
      g.fillRect(gx + tickToX(v, r.start), gy - 3, r.length * v.pxPerTick, 3);
      g.globalAlpha = 1;
    }
    if (head !== undefined) {
      g.fillStyle = "#ffffff";
      g.fillRect(Math.round(gx + tickToX(v, head)) - 3, gy - 6, 7, 6);
    }
    g.restore();

    // Keyboard.
    g.save();
    g.beginPath();
    g.rect(0, gy, gx, H - gy);
    g.clip();
    for (let p = pBottom; p <= pTop; p++) {
      const y = gy + pitchToY(v, p);
      const black = isBlackKey(p);
      g.fillStyle = black ? "#1d1d1f" : "#e9e9ea";
      g.fillRect(0, y, black ? gx * 0.62 : gx, v.keyHeight);
      if (black) {
        g.fillStyle = "#e9e9ea";
        g.fillRect(gx * 0.62, y, gx * 0.38, v.keyHeight);
      }
      g.fillStyle = "rgba(0,0,0,0.25)";
      g.fillRect(0, y + v.keyHeight - 1, gx, 1);
      if (p % 12 === 0 && v.keyHeight >= 9) {
        g.fillStyle = "#333";
        g.font = "9px system-ui, sans-serif";
        g.fillText(noteName(p), gx - 22, y + v.keyHeight - 3);
      }
    }
    g.restore();
    g.fillStyle = col("--ruler", "#333");
    g.fillRect(0, 0, gx, gy);
  };

  const laneNotesValue = (n: RollNote) => {
    const l = laneDrag.current;
    if (l?.kind === "velocity" && l.base?.has(n.key)) {
      const base = l.base.get(n.key)!;
      return Math.max(1, Math.min(127, base + (l.value - l.startValue)));
    }
    return n.note.velocity;
  };

  const drawLane = () => {
    const canvas = laneRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const W = Math.max(0, scroll.w - KEYS_W);
    if (canvas.width !== Math.round(W * dpr) || canvas.height !== Math.round(LANE_H * dpr)) {
      canvas.width = Math.round(W * dpr);
      canvas.height = Math.round(LANE_H * dpr);
      canvas.style.width = `${W}px`;
      canvas.style.height = `${LANE_H}px`;
    }
    const g = canvas.getContext("2d");
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = "#262626";
    g.fillRect(0, 0, W, LANE_H);
    const v = view;
    const t0 = xToTick(v, 0);
    const t1 = xToTick(v, W);
    g.fillStyle = "rgba(255,255,255,0.08)";
    for (const s of map.segments(Math.floor(t0), Math.ceil(t1) + 1)) g.fillRect(Math.round(tickToX(v, s.start)), 0, 1, LANE_H);
    const yOf = (value: number, max: number) => LANE_H - 4 - (value / max) * (LANE_H - 8);
    if (lane === "velocity") {
      const [i0, i1] = visibleRange(roll, t0, t1);
      for (let i = i0; i < i1; i++) {
        const n = roll.notes[i]!;
        const x = Math.round(tickToX(v, n.abs));
        const val = laneNotesValue(n);
        const y = yOf(val, 127);
        const sel = selection.has(n.key);
        g.fillStyle = n.note.muted ? "#777" : sel ? "#ffffff" : n.region.color;
        g.fillRect(x, y, 1.5, LANE_H - 4 - y);
        g.beginPath();
        g.arc(x + 0.75, y, sel ? 3 : 2.2, 0, Math.PI * 2);
        g.fill();
      }
      return;
    }
    const max = LANES.find((l) => l.kind === lane)!.max;
    if (lane === "pitchBend") {
      g.fillStyle = "rgba(255,255,255,0.15)";
      g.fillRect(0, Math.round(yOf(8192, max)), W, 1);
    }
    for (const r of roll.regions) {
      const pts = r.events
        .map((e) => ({ t: r.start + e.tick, v: laneValue(lane, laneChannel, e.bytes) }))
        .filter((p): p is { t: number; v: number } => p.v !== null);
      g.strokeStyle = r.color;
      g.fillStyle = r.color;
      g.lineWidth = 1.5;
      g.beginPath();
      let prev: { x: number; y: number } | null = null;
      for (const p of pts) {
        const x = tickToX(v, p.t);
        const y = yOf(p.v, max);
        if (prev) {
          g.lineTo(x, prev.y);
          g.lineTo(x, y);
        } else g.moveTo(x, y);
        prev = { x, y };
      }
      if (prev) g.lineTo(tickToX(v, r.start + r.length), prev.y);
      g.stroke();
      for (const p of pts) {
        g.beginPath();
        g.arc(tickToX(v, p.t), yOf(p.v, max), 2.2, 0, Math.PI * 2);
        g.fill();
      }
    }
    const l = laneDrag.current;
    if (l?.kind === "draw" && l.points.length) {
      g.strokeStyle = l.erase ? "#ff453a" : "#ffffff";
      g.beginPath();
      l.points.forEach((p, i) => {
        const x = tickToX(v, p.tick);
        const y = l.erase ? LANE_H / 2 : yOf(p.value, max);
        if (i === 0) g.moveTo(x, y);
        else g.lineTo(x, y);
      });
      g.stroke();
    }
  };

  useLayoutEffect(() => {
    drawRef.current = () => {
      draw();
      drawLane();
    };
    // Draw synchronously after every render (size, scroll, project or
    // selection changes); rAF is only used for previews and the playhead.
    drawRef.current();
  });

  // ---------------------------------------------------------- interaction
  const local = (e: { clientX: number; clientY: number }) => {
    const r = canvasRef.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  const regionAt = (tick: number): RollRegion | undefined =>
    [...roll.regions].reverse().find((r) => tick >= r.start && tick < r.start + r.length) ??
    [...roll.regions].reverse().find((r) => tick >= r.start);

  const onDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0) return;
    const { x, y } = local(e);
    scrollRef.current?.focus({ preventScroll: true });
    if (y < RULER_H && x >= KEYS_W) {
      const tick = snapTo(xToTick(view, x - KEYS_W), e.shiftKey);
      void useTransport.getState().send({ type: "locate", tick });
      return;
    }
    if (x < KEYS_W) {
      if (y >= RULER_H) preview({ pitch: yToPitch(view, y - RULER_H), velocity: 100, channel: laneChannel });
      return;
    }
    e.currentTarget.setPointerCapture(e.pointerId);
    const gxp = x - KEYS_W;
    const gyp = y - RULER_H;
    const tick = xToTick(view, gxp);
    const pitch = yToPitch(view, gyp);
    const hit = hitTest(roll, view, gxp, gyp);
    const base: Drag = { kind: "rubber", x, y, tick, pitch, moved: false, dt: 0, dp: 0, copy: false };
    if (tool === "eraser") {
      drag.current = { ...base, kind: "erase", erased: new Set(hit ? [hit.note.key] : []) };
      requestDraw();
      return;
    }
    if (hit) {
      const additive = e.shiftKey || e.ctrlKey || e.metaKey;
      let sel = selection;
      if (additive) {
        sel = new Set(selection);
        if (sel.has(hit.note.key)) sel.delete(hit.note.key);
        else sel.add(hit.note.key);
      } else if (!selection.has(hit.note.key)) sel = new Set([hit.note.key]);
      setSelection(sel);
      if (hit.zone === "body") preview(hit.note.note, hit.note.region.trackId);
      drag.current = { ...base, kind: hit.zone === "body" ? "move" : hit.zone, anchor: hit.note };
      return;
    }
    if (tool === "pencil" || e.detail === 2) {
      const start = floorGrid(tick);
      const region = regionAt(start);
      if (!region || start < region.start) return;
      const len = lastLength.current || stepAt(start);
      drag.current = { ...base, kind: "pencil", tick: start, region, dt: len };
      preview({ pitch, velocity: 100, channel: laneChannel }, region.trackId);
      requestDraw();
      return;
    }
    if (!e.shiftKey) setSelection(new Set());
    drag.current = base;
  };

  const onMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const d = drag.current;
    const { x, y } = local(e);
    if (!d) {
      const hit = x >= KEYS_W && y >= RULER_H && tool !== "eraser" ? hitTest(roll, view, x - KEYS_W, y - RULER_H) : null;
      e.currentTarget.style.cursor = !hit ? (tool === "pencil" ? "crosshair" : "default") : hit.zone === "body" ? "grab" : "ew-resize";
      return;
    }
    if (!d.moved && Math.abs(x - d.x) + Math.abs(y - d.y) < 3) return;
    d.moved = true;
    const tick = xToTick(view, x - KEYS_W);
    const pitch = yToPitch(view, y - RULER_H);
    const free = e.shiftKey;
    switch (d.kind) {
      case "move": {
        const a = d.anchor!;
        d.dt = snapTo(a.abs + (tick - d.tick), free) - a.abs;
        // Keep the group inside its regions.
        const minStart = Math.min(...selected.map((n) => n.abs - n.region.start));
        d.dt = Math.max(-minStart, d.dt);
        const dp = pitch - d.pitch;
        const lo = Math.min(...selected.map((n) => n.note.pitch));
        const hi = Math.max(...selected.map((n) => n.note.pitch));
        const next = Math.max(-lo, Math.min(127 - hi, dp));
        if (next !== d.dp && audition) preview({ ...a.note, pitch: a.note.pitch + next }, a.region.trackId);
        d.dp = next;
        d.copy = e.altKey;
        break;
      }
      case "end": {
        const a = d.anchor!;
        d.dt = Math.max(1 - a.note.length, snapTo(a.end + (tick - d.tick), free) - a.end);
        break;
      }
      case "start": {
        const a = d.anchor!;
        d.dt = snapTo(a.abs + (tick - d.tick), free) - a.abs;
        break;
      }
      case "pencil": {
        const endTick = snapTo(tick, free);
        d.dt = Math.max(free ? 1 : stepAt(d.tick), endTick - d.tick);
        break;
      }
      case "erase": {
        const hit = hitTest(roll, view, x - KEYS_W, y - RULER_H);
        if (hit) d.erased!.add(hit.note.key);
        break;
      }
      case "rubber":
        d.curX = x;
        d.curY = y;
        break;
    }
    requestDraw();
  };

  const onUp = async (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    const finish = () => requestDraw();
    switch (d.kind) {
      case "move": {
        if (!d.moved || (d.dt === 0 && d.dp === 0 && !d.copy)) break;
        const copy = e.altKey;
        const out = await run(moveCommand(selected, d.dt, d.dp, copy));
        if (out && copy) setSelection(new Set(createdKeys(selected, out.createdNotes)));
        break;
      }
      case "start":
      case "end":
        if (d.moved && d.dt !== 0) {
          await run(resizeCommand(selected, d.kind, d.dt));
          if (d.kind === "end" && d.anchor) lastLength.current = Math.max(1, d.anchor.note.length + d.dt);
        }
        break;
      case "pencil": {
        const r = d.region!;
        const length = Math.max(1, d.dt);
        lastLength.current = length;
        const out = await run({
          type: "addNotes",
          regionId: r.id,
          notes: [{ pitch: d.pitch, velocity: 100, channel: laneChannel, start: d.tick - r.start, length }],
        });
        if (out) setSelection(new Set(out.createdNotes.map((id) => noteKey(r.id, id))));
        break;
      }
      case "erase": {
        const notes = roll.notes.filter((n) => d.erased!.has(n.key));
        await run(removeCommand(notes));
        break;
      }
      case "rubber": {
        if (!d.moved || d.curX === undefined || d.curY === undefined) break;
        const hits = notesInRect(
          roll,
          xToTick(view, d.x - KEYS_W),
          xToTick(view, d.curX - KEYS_W),
          yToPitch(view, d.y - RULER_H),
          yToPitch(view, d.curY - RULER_H),
        );
        setSelection((sel) => new Set([...(e.shiftKey ? sel : []), ...hits.map((n) => n.key)]));
        break;
      }
    }
    finish();
  };

  // Ctrl+wheel zooms horizontally around the pointer, Ctrl+Shift+wheel
  // vertically; plain wheel/trackpad scrolls natively.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      const factor = Math.exp(-e.deltaY * 0.002);
      const rect = el.getBoundingClientRect();
      if (e.shiftKey) {
        setKeyH((h) => {
          const next = Math.min(28, Math.max(6, h * factor));
          const y = e.clientY - rect.top - RULER_H;
          el.scrollTop = ((el.scrollTop + y) / h) * next - y;
          return next;
        });
      } else {
        setPpt((p) => {
          const next = Math.min(1.5, Math.max(0.01, p * factor));
          const x = e.clientX - rect.left - KEYS_W;
          el.scrollLeft = ((el.scrollLeft + x) / p) * next - x;
          return next;
        });
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // ------------------------------------------------------------- commands
  const targets = (all = false) => {
    const sel = all || selected.length === 0 ? roll.notes : selected;
    const regions = new Map<string, number[]>();
    for (const n of sel) regions.set(n.region.id, [...(regions.get(n.region.id) ?? []), n.note.id]);
    return [...regions].map(([regionId, ids]) => ({ regionId, ids: selected.length === 0 || all ? undefined : ids }));
  };
  const transform = (t: NoteTransform) => {
    if (!roll.notes.length) return;
    void run({ type: "transformNotes", targets: targets(), transform: t });
  };
  const quantize = () => transform({ type: "quantize", grid, strength });
  const nudge = (dt: number, dp: number) => void run(moveCommand(selected, dt, dp, false));
  const duplicate = async () => {
    if (!selected.length) return;
    const span = Math.max(...selected.map((n) => n.end)) - Math.min(...selected.map((n) => n.abs));
    const out = await run(moveCommand(selected, span, 0, true));
    if (out) setSelection(new Set(createdKeys(selected, out.createdNotes)));
  };
  const toggleMute = () => {
    if (!selected.length) return;
    transform({ type: "mute", muted: !selected.every((n) => n.note.muted) });
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.target as HTMLElement).closest("input, select, textarea")) return;
    const mod = e.ctrlKey || e.metaKey;
    const step = e.shiftKey ? ticksPerBar(map.timeSignatureAt(selected[0]?.abs ?? 0)) : stepAt(selected[0]?.abs ?? 0);
    const handled = (() => {
      if (mod && e.key.toLowerCase() === "a") return setSelection(new Set(roll.notes.map((n) => n.key))), true;
      if (mod && e.key.toLowerCase() === "d") return void duplicate(), true;
      if (mod) return false;
      switch (e.key) {
        case "Delete":
        case "Backspace":
          return void run(removeCommand(selected)), true;
        case "ArrowLeft":
          return selected.length > 0 && (nudge(-step, 0), true);
        case "ArrowRight":
          return selected.length > 0 && (nudge(step, 0), true);
        case "ArrowUp":
          return selected.length > 0 && (nudge(0, e.shiftKey ? 12 : 1), true);
        case "ArrowDown":
          return selected.length > 0 && (nudge(0, e.shiftKey ? -12 : -1), true);
        case "q":
        case "Q":
          return quantize(), true;
        case "m":
        case "M":
          return toggleMute(), true;
        case "Escape":
          return setSelection(new Set()), true;
        case "1":
          return setTool("pointer"), true;
        case "2":
          return setTool("pencil"), true;
        case "3":
          return setTool("eraser"), true;
      }
      return false;
    })();
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  // -------------------------------------------------------------- lane
  const laneDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    e.currentTarget.setPointerCapture(e.pointerId);
    const max = LANES.find((l) => l.kind === lane)!.max;
    const value = Math.round(Math.max(0, Math.min(1, (LANE_H - 4 - y) / (LANE_H - 8))) * max);
    if (lane === "velocity") {
      const tick = xToTick(view, x);
      const [i0, i1] = visibleRange(roll, tick - 6 / view.pxPerTick, tick + 6 / view.pxPerTick);
      let best: RollNote | null = null;
      for (let i = i0; i < i1; i++) {
        const n = roll.notes[i]!;
        if (Math.abs(tickToX(view, n.abs) - x) <= 5 && (!best || selection.has(n.key))) best = n;
      }
      if (!best) return;
      const group = selection.has(best.key) ? selected : [best];
      laneDrag.current = {
        kind: "velocity",
        notes: group,
        base: new Map(group.map((n) => [n.key, n.note.velocity])),
        points: [],
        startValue: best.note.velocity,
        y,
        value,
        erase: false,
      };
      laneDrag.current.value = best.note.velocity;
    } else {
      laneDrag.current = { kind: "draw", points: [{ tick: snapTo(xToTick(view, x), e.shiftKey), value }], startValue: value, y, value, erase: e.altKey };
    }
    requestDraw();
  };
  const laneMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const l = laneDrag.current;
    if (!l) return;
    const r = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    const max = LANES.find((k) => k.kind === lane)!.max;
    if (l.kind === "velocity") {
      l.value = Math.round(Math.max(1, Math.min(127, l.startValue + ((l.y - y) / (LANE_H - 8)) * 127)));
    } else {
      const tick = snapTo(xToTick(view, x), e.shiftKey);
      const value = Math.round(Math.max(0, Math.min(1, (LANE_H - 4 - y) / (LANE_H - 8))) * max);
      const last = l.points[l.points.length - 1];
      if (last && last.tick === tick) last.value = value;
      else l.points.push({ tick, value });
    }
    requestDraw();
  };
  const laneUp = async () => {
    const l = laneDrag.current;
    laneDrag.current = null;
    if (!l) return;
    if (l.kind === "velocity" && l.notes && l.base) {
      const delta = l.value - l.startValue;
      if (delta !== 0) {
        const base = l.base;
        await run(updateCommand(l.notes, (n, from) => ({ ...n, velocity: Math.max(1, Math.min(127, (base.get(from.key) ?? n.velocity) + delta)) })));
      }
    } else if (l.kind === "draw" && lane !== "velocity" && l.points.length) {
      const ticks = l.points.map((p) => p.tick);
      const from = Math.min(...ticks);
      const to = Math.max(...ticks);
      const region = regionAt(from);
      if (region) {
        const rel = (t: number) => Math.max(0, Math.min(region.length, t - region.start));
        const points = l.erase
          ? []
          : [...new Map(l.points.map((p) => [rel(p.tick), p.value])).entries()]
              .sort((a, b) => a[0] - b[0])
              .map(([tick, value]) => ({ tick, value }));
        await run({
          type: "editControllerLane",
          regionId: region.id,
          lane: laneOf(lane, laneChannel),
          from: rel(from),
          to: Math.min(region.length, rel(to) + 1),
          points,
        });
      }
    }
    requestDraw();
  };

  // ------------------------------------------------------------- render
  const width = KEYS_W + end * ppt;
  const height = RULER_H + 128 * keyH;
  const single1 = selected.length === 1 ? selected[0]! : null;
  const editSelected = (patch: (n: MidiNote) => MidiNote) => void run(updateCommand(selected, patch));
  const functions: MenuEntry[] = [
    { id: "q", label: `Quantize to ${GRID_LABELS[grid.division]}${grid.triplet ? " triplets" : ""} (${strength}%)`, shortcut: "Q", onSelect: quantize },
    { id: "uq", label: "Unquantize", onSelect: () => transform({ type: "unquantize" }) },
    { id: "hum", label: "Humanize (±10 ticks, ±8 velocity)", onSelect: () => transform({ type: "humanize", timing: 10, velocity: 8, length: 0, seed: Math.floor(Math.random() * 2 ** 31) }) },
    { id: "leg", label: "Legato", onSelect: () => transform({ type: "legato", gap: 0 }) },
    "separator",
    { id: "t+1", label: "Transpose +1", onSelect: () => transform({ type: "transpose", semitones: 1 }) },
    { id: "t-1", label: "Transpose −1", onSelect: () => transform({ type: "transpose", semitones: -1 }) },
    { id: "t+12", label: "Transpose +12", onSelect: () => transform({ type: "transpose", semitones: 12 }) },
    { id: "t-12", label: "Transpose −12", onSelect: () => transform({ type: "transpose", semitones: -12 }) },
    "separator",
    { id: "mute", label: "Mute / Unmute Notes", shortcut: "M", disabled: !selected.length, onSelect: toggleMute },
    { id: "dup", label: "Duplicate", shortcut: "Ctrl+D", disabled: !selected.length, onSelect: () => void duplicate() },
    {
      id: "split",
      label: "Split Chord Voices to New Tracks",
      disabled: roll.regions.length !== 1,
      onSelect: () => void run({ type: "splitChordVoices", regionId: primary.id, ids: selected.length ? selected.map((n) => n.note.id) : undefined }),
    },
    { id: "all", label: "Select All", shortcut: "Ctrl+A", onSelect: () => setSelection(new Set(roll.notes.map((n) => n.key))) },
    { id: "del", label: "Delete", danger: true, disabled: !selected.length, onSelect: () => void run(removeCommand(selected)) },
  ];

  return (
    <div className="pr" data-testid="piano-roll">
      <div className="pr__bar">
        <span className="panel-title" title={roll.regions.length > 1 ? "Editing several regions" : primary.id}>
          {roll.regions.length > 1 ? `${roll.regions.length} regions` : primaryTrack?.name ?? "MIDI"}
        </span>
        <div className="segmented" role="group" aria-label="Tool">
          {(
            [
              ["pointer", "pointer", "Pointer (1): select, drag, resize; Alt-drag copies; Shift disables snap"],
              ["pencil", "pencil", "Pencil (2): draw notes; drag to set length"],
              ["eraser", "eraser", "Eraser (3): click or drag over notes"],
            ] as const
          ).map(([id, icon, title]) => (
            <button key={id} type="button" className={tool === id ? "is-on" : ""} aria-pressed={tool === id} title={title} onClick={() => setTool(id)}>
              <Icon name={icon} size={13} />
            </button>
          ))}
        </div>
        <IconButton icon="snap" label={snap ? "Snap on" : "Snap off"} active={snap} size={13} onClick={() => setSnap(!snap)} />
        <select className="pr__select" aria-label="Grid" value={grid.division} onChange={(e) => setGrid({ ...grid, division: e.target.value as GridSpec["division"] })}>
          {DIVISIONS.map((d) => (
            <option key={d} value={d}>
              {GRID_LABELS[d]}
            </option>
          ))}
        </select>
        <label className="pr__check" title="Triplet grid">
          <input type="checkbox" checked={grid.triplet} onChange={(e) => setGrid({ ...grid, triplet: e.target.checked })} />
          3
        </label>
        <NumberField label="Swing" value={grid.swing} min={0} max={100} step={5} suffix="%sw" onCommit={(swing) => setGrid({ ...grid, swing })} />
        <NumberField label="Quantize strength" value={strength} min={0} max={100} step={5} suffix="%" onCommit={setStrength} />
        <button type="button" className="pr__btn" title="Quantize selection (or all notes) to the grid (Q)" onClick={quantize}>
          Q
        </button>
        <button type="button" className="pr__btn" onClick={(e) => setMenu({ x: e.clientX, y: e.clientY })}>
          Functions <Icon name="chevronDown" size={10} />
        </button>
        <IconButton icon="audition" label={audition ? "Audition notes on (built-in instrument / MIDI out)" : "Audition notes off"} active={audition} size={13} onClick={() => setAudition(!audition)} />
        <label className="pr__check" title={`Highlight the ${project.transport.key.tonic} ${project.transport.key.mode} scale`}>
          <input type="checkbox" checked={scale} onChange={(e) => setScale(e.target.checked)} />
          Scale
        </label>
        <span className="pr__sel">
          {single1 ? (
            <>
              {noteName(single1.note.pitch)} · {map.formatPosition(single1.abs)}
            </>
          ) : (
            `${selected.length}/${roll.notes.length} notes`
          )}
        </span>
        {selected.length > 0 && (
          <>
            <NumberField
              label="Velocity"
              value={single1 ? single1.note.velocity : Math.round(selected.reduce((a, n) => a + n.note.velocity, 0) / selected.length)}
              min={1}
              max={127}
              suffix="vel"
              onCommit={(velocity) => editSelected((n) => ({ ...n, velocity }))}
            />
            <NumberField label="MIDI channel" value={(single1?.note.channel ?? selected[0]!.note.channel) + 1} min={1} max={16} suffix="ch" onCommit={(c) => editSelected((n) => ({ ...n, channel: c - 1 }))} />
            {single1 && <NumberField label="Length (ticks)" value={single1.note.length} min={1} max={3840 * 64} step={10} suffix="tk" onCommit={(length) => editSelected((n) => ({ ...n, length }))} />}
          </>
        )}
        <div className="pr__bar-end">
          <IconButton icon="zoomOut" label="Zoom out (Ctrl+wheel)" size={13} onClick={() => setPpt((p) => Math.max(0.01, p / 1.4))} />
          <IconButton icon="zoomIn" label="Zoom in (Ctrl+wheel)" size={13} onClick={() => setPpt((p) => Math.min(1.5, p * 1.4))} />
          {roll.notes.some((n) => n.note.quantizeOffset !== 0) && <Badge tone="accent">Quantized</Badge>}
        </div>
      </div>
      <div
        className="pr__scroll"
        ref={scrollRef}
        tabIndex={0}
        aria-label="Piano roll notes"
        onScroll={() => {
          sync();
          requestDraw();
        }}
        onKeyDown={onKeyDown}
      >
        <div className="pr__spacer" style={{ width, height }}>
          <canvas
            ref={canvasRef}
            className="pr__canvas"
            onPointerDown={onDown}
            onPointerMove={onMove}
            onPointerUp={(e) => void onUp(e)}
            onPointerCancel={() => {
              drag.current = null;
              requestDraw();
            }}
            onDoubleClick={(e) => e.preventDefault()}
          />
        </div>
      </div>
      <div className="pr__lane">
        <div className="pr__lane-head">
          <select className="pr__select" aria-label="Controller lane" value={lane} onChange={(e) => setLane(e.target.value as LaneKind)}>
            {LANES.map((l) => (
              <option key={l.kind} value={l.kind}>
                {l.label}
              </option>
            ))}
          </select>
          {lane !== "velocity" && (
            <NumberField label="Lane MIDI channel" value={laneChannel + 1} min={1} max={16} suffix="ch" onCommit={(c) => setLaneChannel(c - 1)} />
          )}
          <span className="hint">{lane === "velocity" ? "Drag a stalk" : "Draw; Alt-drag erases"}</span>
        </div>
        <canvas ref={laneRef} className="pr__lane-canvas" onPointerDown={laneDown} onPointerMove={laneMove} onPointerUp={() => void laneUp()} />
      </div>
      {menu && <Menu at={menu} entries={functions} onClose={() => setMenu(null)} />}
    </div>
  );
}

export function AudioEditor() {
  const project = useProject((s) => s.view?.project);
  const regionIds = useUi((s) => s.selection.regionIds);
  const region = selectedRegion(project, regionIds, "audio");
  return (
    <div className="editor-empty">
      <Icon name="waveform" size={22} />
      {region && region.content.type === "audio" ? (
        <>
          <p>
            {region.name}: {region.content.media ? (project?.media.find((m) => m.id === (region.content as { media: string }).media)?.path ?? "file") : "placeholder region without an audio file"}
          </p>
          <span>Waveforms, trimming, fades, gain and looping are edited in the arrangement and inspector. A sample-level audio editor is not available yet.</span>
        </>
      ) : (
        <>
          <p>Audio editor</p>
          <span>Select an audio region. Sample-level editing is not available yet; use the arrangement and inspector.</span>
        </>
      )}
    </div>
  );
}
