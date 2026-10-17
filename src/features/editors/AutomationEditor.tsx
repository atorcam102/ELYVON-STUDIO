// Automation lane editor (Phase 10). Draws a channel's (or region's) lane with
// the same evaluation as the engine and turns every gesture into one native
// edit: add, move (Alt copies), curve changes, delete, copy/paste and
// duplicate. Shift disables snapping.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import type { AutomationLane } from "../../bindings/AutomationLane";
import type { AutomationMode } from "../../bindings/AutomationMode";
import type { AutomationParam } from "../../bindings/AutomationParam";
import type { AutomationPoint } from "../../bindings/AutomationPoint";
import type { AutomationTarget } from "../../bindings/AutomationTarget";
import type { ChannelRef } from "../../bindings/ChannelRef";
import type { Curve } from "../../bindings/Curve";
import type { GridSpec } from "../../bindings/GridSpec";
import type { Project } from "../../bindings/Project";
import { Icon } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import {
  MODES,
  channelAutomation,
  formatParamValue,
  fromNorm,
  lanePieces,
  paramKey,
  paramLabel,
  paramSpec,
  piecesValue,
  sameChannel,
  sameParam,
  staticValue,
  toNorm,
  writableParams,
} from "../../lib/automation";
import { ticksPerBar } from "../../lib/music";
import { GRID_LABELS, snapTick, TempoMap } from "../../lib/tempoMap";
import { notify } from "../../state/notifications";
import { useProject } from "../../state/project";
import { isRolling, useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";

const PAD = 10;
const HIT = 6;
const DIVISIONS: GridSpec["division"][] = ["bar", "quarter", "eighth", "sixteenth", "thirtySecond"];

const CURVES: { label: string; curve: Curve }[] = [
  { label: "Linear", curve: { type: "linear" } },
  { label: "Step (hold)", curve: { type: "step" } },
  { label: "Ease in", curve: { type: "bezier", c1: 0, c2: 0.1 } },
  { label: "Ease out", curve: { type: "bezier", c1: 0.9, c2: 1 } },
  { label: "S-curve", curve: { type: "bezier", c1: 0, c2: 1 } },
];

interface Clip {
  dt: number;
  value: number;
  curve: Curve;
}
let clipboard: Clip[] = [];

function channelName(project: Project, c: ChannelRef): string {
  if (c.type === "track") return project.tracks.find((t) => t.id === c.id)?.name ?? "Track";
  if (c.type === "aux") return project.mixer.auxes.find((a) => a.id === c.id)?.name ?? "Aux";
  return "Stereo Out";
}

function channelKey(c: ChannelRef): string {
  return c.type === "master" ? "master" : `${c.type}:${c.id}`;
}

function channels(project: Project): ChannelRef[] {
  return [
    ...project.tracks.filter((t) => t.kind !== "midi").map((t) => ({ type: "track", id: t.id }) as ChannelRef),
    ...project.mixer.auxes.map((a) => ({ type: "aux", id: a.id }) as ChannelRef),
    { type: "master" },
  ];
}

export function AutomationEditor() {
  const project = useProject((s) => s.view?.project);
  const trackIds = useUi((s) => s.selection.trackIds);
  const regionIds = useUi((s) => s.selection.regionIds);
  const [picked, setPicked] = useState<ChannelRef | null>(null);
  const selectedTrack = trackIds[trackIds.length - 1];
  // Follow the arrangement selection until another channel is picked here.
  useEffect(() => setPicked(null), [selectedTrack]);
  if (!project) return null;
  const all = channels(project);
  const channel: ChannelRef | undefined =
    (picked && all.find((c) => sameChannel(c, picked))) ??
    all.find((c) => c.type === "track" && c.id === selectedTrack) ??
    all.find((c) => c.type === "track");
  if (!channel) {
    return (
      <div className="editor-empty">
        <Icon name="mixer" size={22} />
        <p>Add an audio or instrument track to automate its mix.</p>
        <span>External MIDI tracks have no audio channel to automate.</span>
      </div>
    );
  }
  const track = channel.type === "track" ? project.tracks.find((t) => t.id === channel.id) : undefined;
  const region = track?.regions.find((r) => regionIds.includes(r.id));
  return <LaneEditor key={channelKey(channel)} project={project} channel={channel} region={region?.id ?? null} channels={all} onPick={setPicked} />;
}

interface Drag {
  kind: "move" | "rubber";
  x: number;
  y: number;
  curX: number;
  curY: number;
  moved: boolean;
  anchor?: AutomationPoint;
  copy: boolean;
}

function LaneEditor({
  project,
  channel,
  region: regionId,
  channels: all,
  onPick,
}: {
  project: Project;
  channel: ChannelRef;
  region: string | null;
  channels: ChannelRef[];
  onPick: (c: ChannelRef) => void;
}) {
  const apply = useProject((s) => s.apply);
  const pxPerQuarter = useUi((s) => s.pxPerQuarter);
  const cursor = useUi((s) => s.cursorTicks);
  const snapshot = useTransport((s) => s.snapshot);
  const map = useMemo(() => TempoMap.fromTransport(project.transport), [project.transport]);
  const params = useMemo(() => writableParams(project, channel), [project, channel]);
  const auto = channelAutomation(project, channel);
  const [paramK, setParamK] = useState<string>(() => {
    const withLane = auto?.lanes.find((l) => l.points.length > 0);
    return withLane ? paramKey(withLane.param) : "volume";
  });
  const param: AutomationParam = params.find((p) => paramKey(p) === paramK) ?? params[0] ?? { type: "volume" };
  const [useRegion, setUseRegion] = useState(false);
  const regionObj = regionId ? project.tracks.flatMap((t) => t.regions).find((r) => r.id === regionId) : undefined;
  const scopeRegion = useRegion && regionObj ? regionObj : undefined;
  const target: AutomationTarget = { channel, region: scopeRegion?.id, param };
  const lane: AutomationLane | undefined = scopeRegion
    ? scopeRegion.automation.find((l) => sameParam(l.param, param))
    : auto?.lanes.find((l) => sameParam(l.param, param));
  const base = scopeRegion?.start ?? 0;
  const spec = paramSpec(param);
  const [grid, setGrid] = useState<GridSpec>({ division: "sixteenth", triplet: false, swing: 0 });
  const [snapOn, setSnapOn] = useState(true);
  const [selection, setSelection] = useState<Set<number>>(() => new Set());
  const [menu, setMenu] = useState<{ x: number; y: number; entries: MenuEntry[] } | null>(null);
  const [hover, setHover] = useState<{ tick: number; value: number } | null>(null);
  const [view, setView] = useState({ x: 0, w: 600, h: 160 });
  const scrollRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const drag = useRef<Drag | null>(null);
  const [, force] = useState(0);
  const ppt = pxPerQuarter / 960;

  useEffect(() => setSelection(new Set()), [paramK, useRegion, regionId]);

  const end = Math.max(map.barStart(project.lengthBars), project.tracks.flatMap((t) => t.regions).reduce((m, r) => Math.max(m, r.start + r.length), 0) + ticksPerBar(project.transport.timeSignature));
  const width = Math.ceil(end * ppt) + 200;

  const sync = useCallback(() => {
    const el = scrollRef.current;
    if (el) setView({ x: el.scrollLeft, w: el.clientWidth, h: el.clientHeight });
  }, []);
  useLayoutEffect(() => {
    sync();
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(sync);
    ro.observe(el);
    return () => ro.disconnect();
  }, [sync]);

  const xOf = useCallback((tick: number) => tick * ppt - view.x, [ppt, view.x]);
  const tickOf = useCallback((x: number) => Math.max(0, (x + view.x) / ppt), [ppt, view.x]);
  const yOfNorm = useCallback((n: number) => PAD + (1 - n) * (view.h - 2 * PAD), [view.h]);
  const normOfY = (y: number) => Math.min(1, Math.max(0, 1 - (y - PAD) / Math.max(1, view.h - 2 * PAD)));
  const snap = (t: number, free: boolean) => (snapOn && !free ? snapTick(map, Math.max(0, t), grid) : Math.max(0, Math.round(t)));
  const points = lane?.points ?? [];
  const absTick = (p: AutomationPoint) => p.tick + base;
  const staticV = staticValue(project, channel, param) ?? spec.min;
  const playhead = isRolling(snapshot) && snapshot ? snapshot.ticks : cursor;

  const run = useCallback(
    async (cmd: Parameters<typeof apply>[0]) => {
      const out = await apply(cmd);
      if (out?.createdPoints.length) setSelection(new Set(out.createdPoints));
      return out;
    },
    [apply],
  );

  // ---- drawing ----
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const W = view.w;
    const H = view.h;
    if (canvas.width !== Math.round(W * dpr) || canvas.height !== Math.round(H * dpr)) {
      canvas.width = Math.round(W * dpr);
      canvas.height = Math.round(H * dpr);
      canvas.style.width = `${W}px`;
      canvas.style.height = `${H}px`;
    }
    const g = canvas.getContext("2d");
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = "#232323";
    g.fillRect(0, 0, W, H);
    const t0 = tickOf(0);
    const t1 = tickOf(W);
    for (const s of map.segments(Math.floor(t0), Math.ceil(t1) + 1)) {
      g.fillStyle = "rgba(255,255,255,0.08)";
      g.fillRect(Math.round(xOf(s.start)), 0, 1, H);
    }
    // Value guides: top, middle, bottom.
    g.fillStyle = "rgba(255,255,255,0.05)";
    for (const n of [0, 0.5, 1]) g.fillRect(0, Math.round(yOfNorm(n)), W, 1);
    // Region scope: shade outside the region.
    if (scopeRegion) {
      g.fillStyle = "rgba(0,0,0,0.35)";
      g.fillRect(0, 0, Math.max(0, xOf(scopeRegion.start)), H);
      g.fillRect(xOf(scopeRegion.start + scopeRegion.length), 0, W, H);
    } else if (channel.type === "track") {
      // Regions whose own lane replaces this one.
      const track = project.tracks.find((t) => t.id === channel.id);
      for (const r of track?.regions ?? []) {
        if (r.muted || !r.automation.some((l) => sameParam(l.param, param) && l.points.length)) continue;
        g.fillStyle = "rgba(201,154,34,0.12)";
        g.fillRect(xOf(r.start), 0, r.length * ppt, H);
      }
    }
    const d = drag.current;
    const moving = d?.kind === "move" && d.moved && d.anchor ? d : null;
    const dt = moving && moving.anchor ? snap(absTick(moving.anchor) + (moving.curX - moving.x) / ppt, false) - absTick(moving.anchor) : 0;
    const dn = moving ? -(moving.curY - moving.y) / Math.max(1, view.h - 2 * PAD) : 0;
    const shown: AutomationPoint[] = points.map((p) =>
      moving && selection.has(p.id) ? { ...p, tick: p.tick + dt, value: fromNorm(spec, toNorm(spec, p.value) + dn) } : p,
    );
    if (moving?.copy) for (const p of points) if (selection.has(p.id)) shown.push(p);
    shown.sort((a, b) => a.tick - b.tick);
    const pieces = shown.length ? lanePieces({ param, points: shown }, base) : null;
    g.strokeStyle = lane?.points.length ? "#f2c14e" : "rgba(242,193,78,0.45)";
    g.lineWidth = 1.5;
    g.setLineDash(pieces ? [] : [4, 4]);
    g.beginPath();
    const from = scopeRegion ? Math.max(0, xOf(scopeRegion.start)) : 0;
    const to = scopeRegion ? Math.min(W, xOf(scopeRegion.start + scopeRegion.length)) : W;
    for (let x = Math.floor(from); x <= Math.ceil(to); x++) {
      const t = tickOf(x);
      const n = pieces ? (piecesValue(pieces, t) ?? toNorm(spec, staticV)) : toNorm(spec, staticV);
      const y = yOfNorm(n);
      if (x === Math.floor(from)) g.moveTo(x, y);
      else g.lineTo(x, y);
    }
    g.stroke();
    g.setLineDash([]);
    for (const p of shown) {
      const x = xOf(p.tick + base);
      if (x < -HIT || x > W + HIT) continue;
      const y = yOfNorm(toNorm(spec, p.value));
      const sel = selection.has(p.id);
      g.fillStyle = sel ? "#ffffff" : "#f2c14e";
      g.beginPath();
      g.arc(x, y, sel ? 4 : 3.2, 0, Math.PI * 2);
      g.fill();
      if (p.curve.type !== "linear") {
        g.fillStyle = "rgba(255,255,255,0.55)";
        g.font = "9px system-ui";
        g.fillText(p.curve.type === "step" ? "▭" : "~", x + 5, y - 5);
      }
    }
    if (d?.kind === "rubber" && d.moved) {
      g.strokeStyle = "rgba(255,255,255,0.7)";
      g.strokeRect(Math.min(d.x, d.curX) + 0.5, Math.min(d.y, d.curY) + 0.5, Math.abs(d.curX - d.x), Math.abs(d.curY - d.y));
    }
    const px = xOf(playhead);
    if (px >= 0 && px <= W) {
      g.fillStyle = "#ffffff";
      g.fillRect(Math.round(px), 0, 1, H);
    }
  });

  // ---- hit testing and gestures ----
  const pointAt = (x: number, y: number): AutomationPoint | undefined => {
    let best: { p: AutomationPoint; d: number } | undefined;
    for (const p of points) {
      const dx = xOf(p.tick + base) - x;
      const dy = yOfNorm(toNorm(spec, p.value)) - y;
      const dist = Math.hypot(dx, dy);
      if (dist <= HIT && (!best || dist < best.d)) best = { p, d: dist };
    }
    return best?.p;
  };

  const local = (e: { clientX: number; clientY: number }) => {
    const r = canvasRef.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  const relTick = (abs: number) => {
    const t = abs - base;
    return scopeRegion ? Math.min(scopeRegion.length, Math.max(0, t)) : Math.max(0, t);
  };

  const onDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0) return;
    const { x, y } = local(e);
    canvasRef.current?.focus();
    canvasRef.current?.setPointerCapture(e.pointerId);
    const hit = pointAt(x, y);
    if (hit) {
      let sel = selection;
      if (e.ctrlKey || e.metaKey) {
        sel = new Set(selection);
        if (sel.has(hit.id)) sel.delete(hit.id);
        else sel.add(hit.id);
      } else if (!selection.has(hit.id)) sel = new Set([hit.id]);
      setSelection(sel);
      drag.current = { kind: "move", x, y, curX: x, curY: y, moved: false, anchor: hit, copy: e.altKey };
    } else {
      drag.current = { kind: "rubber", x, y, curX: x, curY: y, moved: false, copy: false };
    }
  };

  const onMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const { x, y } = local(e);
    setHover({ tick: tickOf(x), value: fromNorm(spec, normOfY(y)) });
    const d = drag.current;
    if (!d) return;
    if (!d.moved && Math.hypot(x - d.x, y - d.y) < 3) return;
    d.moved = true;
    d.curX = x;
    d.curY = y;
    force((n) => n + 1);
  };

  const onUp = async (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    const free = e.shiftKey;
    if (d.kind === "move" && d.moved && d.anchor) {
      const dt = snap(absTick(d.anchor) + (d.curX - d.x) / ppt, free) - absTick(d.anchor);
      const dn = -(d.curY - d.y) / Math.max(1, view.h - 2 * PAD);
      await run({ type: "moveAutomationPoints", target, ids: [...selection], deltaTicks: Math.round(dt), deltaValue: dn, copy: d.copy });
    } else if (d.kind === "rubber" && d.moved) {
      const [x0, x1] = [Math.min(d.x, d.curX), Math.max(d.x, d.curX)];
      const [y0, y1] = [Math.min(d.y, d.curY), Math.max(d.y, d.curY)];
      const inside = points.filter((p) => {
        const px = xOf(p.tick + base);
        const py = yOfNorm(toNorm(spec, p.value));
        return px >= x0 && px <= x1 && py >= y0 && py <= y1;
      });
      setSelection(new Set(e.ctrlKey || e.metaKey ? [...selection, ...inside.map((p) => p.id)] : inside.map((p) => p.id)));
    } else if (d.kind === "rubber") {
      const abs = snap(tickOf(d.x), free);
      if (scopeRegion && (abs < scopeRegion.start || abs > scopeRegion.start + scopeRegion.length)) {
        notify("info", "Region automation lies inside the selected region.");
        return;
      }
      await run({ type: "addAutomationPoints", target, points: [{ tick: relTick(abs), value: fromNorm(spec, normOfY(d.y)) }] });
    }
    force((n) => n + 1);
  };

  const removeSelected = () => {
    if (selection.size) void run({ type: "removeAutomationPoints", target, ids: [...selection] });
    setSelection(new Set());
  };

  const copySelected = () => {
    const sel = points.filter((p) => selection.has(p.id));
    if (!sel.length) return;
    const first = sel[0]!.tick;
    clipboard = sel.map((p) => ({ dt: p.tick - first, value: p.value, curve: p.curve }));
  };

  const paste = () => {
    if (!clipboard.length) return;
    const at = relTick(snap(playhead, false));
    void run({ type: "addAutomationPoints", target, points: clipboard.map((c) => ({ tick: at + c.dt, value: c.value, curve: c.curve })) });
  };

  const duplicate = () => {
    const sel = points.filter((p) => selection.has(p.id));
    if (!sel.length) return;
    const span = Math.max(sel[sel.length - 1]!.tick - sel[0]!.tick, 240);
    void run({ type: "moveAutomationPoints", target, ids: sel.map((p) => p.id), deltaTicks: span, deltaValue: 0, copy: true });
  };

  const setCurve = (curve: Curve) => {
    const sel = points.filter((p) => selection.has(p.id));
    if (sel.length) void run({ type: "updateAutomationPoints", target, points: sel.map((p) => ({ ...p, curve })) });
  };

  const onKey = (e: KeyboardEvent<HTMLCanvasElement>) => {
    const mod = e.ctrlKey || e.metaKey;
    if (e.key === "Delete" || e.key === "Backspace") removeSelected();
    else if (mod && e.key.toLowerCase() === "a") setSelection(new Set(points.map((p) => p.id)));
    else if (mod && e.key.toLowerCase() === "c") copySelected();
    else if (mod && e.key.toLowerCase() === "v") paste();
    else if (mod && e.key.toLowerCase() === "d") duplicate();
    else if (e.key === "Escape") setSelection(new Set());
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  const onContext = (e: React.MouseEvent<HTMLCanvasElement>) => {
    e.preventDefault();
    const { x, y } = local(e);
    const hit = pointAt(x, y);
    if (hit && !selection.has(hit.id)) setSelection(new Set([hit.id]));
    const any = hit !== undefined || selection.size > 0;
    setMenu({
      x: e.clientX,
      y: e.clientY,
      entries: [
        { heading: "Curve to the next point" },
        ...CURVES.map((c) => ({ id: c.label, label: c.label, disabled: !any || spec.discrete, onSelect: () => setCurve(c.curve) })),
        "separator",
        { id: "copy", label: "Copy", shortcut: "Ctrl+C", disabled: !any, onSelect: copySelected },
        { id: "paste", label: "Paste at playhead", shortcut: "Ctrl+V", disabled: !clipboard.length, onSelect: paste },
        { id: "dup", label: "Duplicate", shortcut: "Ctrl+D", disabled: !any, onSelect: duplicate },
        { id: "del", label: "Delete", danger: true, disabled: !any, onSelect: removeSelected },
      ],
    });
  };

  const setMode = (mode: AutomationMode) => void apply({ type: "setAutomationMode", channel, mode });
  const hasLane = (p: AutomationParam) =>
    auto?.lanes.some((l) => sameParam(l.param, p) && l.points.length > 0) ||
    (channel.type === "track" && project.tracks.find((t) => t.id === channel.id)?.regions.some((r) => r.automation.some((l) => sameParam(l.param, p) && l.points.length)));

  return (
    <div className="pr automation" data-testid="automation-editor">
      <div className="pr__bar">
        <select className="pr__select" aria-label="Automated channel" value={channelKey(channel)} onChange={(e) => onPick(all.find((c) => channelKey(c) === e.target.value)!)}>
          {all.map((c) => (
            <option key={channelKey(c)} value={channelKey(c)}>
              {channelName(project, c)}
            </option>
          ))}
        </select>
        <select className="pr__select" aria-label="Automated parameter" value={paramKey(param)} onChange={(e) => setParamK(e.target.value)}>
          {params.map((p) => (
            <option key={paramKey(p)} value={paramKey(p)}>
              {hasLane(p) ? "● " : ""}
              {paramLabel(project, channel, p)}
            </option>
          ))}
        </select>
        <div className="segmented" role="group" aria-label="Automation mode">
          {MODES.map((m) => (
            <button key={m.mode} type="button" className={auto?.mode === m.mode ? "is-on" : ""} aria-pressed={auto?.mode === m.mode} title={m.title} onClick={() => setMode(m.mode)}>
              {m.label}
            </button>
          ))}
        </div>
        {channel.type === "track" && (
          <label className="pr__check" title={regionObj ? `Edit the lane of region “${regionObj.name}” (it moves with the region)` : "Select a region on this track to edit its own lane"}>
            <input type="checkbox" checked={useRegion && !!regionObj} disabled={!regionObj} onChange={(e) => setUseRegion(e.target.checked)} />
            Region lane
          </label>
        )}
        <label className="pr__check" title="Snap points to the grid (hold Shift while dragging to place freely)">
          <input type="checkbox" checked={snapOn} onChange={(e) => setSnapOn(e.target.checked)} />
          Snap
        </label>
        <select className="pr__select" aria-label="Automation grid" value={grid.division} onChange={(e) => setGrid({ ...grid, division: e.target.value as GridSpec["division"] })}>
          {DIVISIONS.map((d) => (
            <option key={d} value={d}>
              {GRID_LABELS[d]}
            </option>
          ))}
        </select>
        <button type="button" className="pr__btn" disabled={!lane || lane.points.length < 3} title="Remove points that do not change the curve" onClick={() => void apply({ type: "simplifyAutomation", target, tolerance: 0.005 })}>
          Simplify
        </button>
        <button type="button" className="pr__btn" disabled={!lane} title="Delete this lane" onClick={() => void apply({ type: "removeAutomationLane", target })}>
          Delete lane
        </button>
        <span className="pr__sel">
          {selection.size ? `${selection.size} selected · ` : ""}
          {lane ? `${lane.points.length} points` : "No automation — click to add"}
        </span>
        <div className="pr__bar-end">
          <span className="hint" aria-live="polite">
            {hover ? `${map.formatPosition(Math.round(hover.tick))} · ${formatParamValue(param, hover.value)}` : formatParamValue(param, staticV)}
          </span>
        </div>
      </div>
      <div className="pr__scroll" ref={scrollRef} onScroll={sync}>
        <div className="pr__spacer" style={{ width, height: "100%" }}>
          <canvas
            ref={canvasRef}
            className="pr__canvas automation__canvas"
            tabIndex={0}
            aria-label={`${paramLabel(project, channel, param)} automation of ${channelName(project, channel)}`}
            onPointerDown={onDown}
            onPointerMove={onMove}
            onPointerUp={(e) => void onUp(e)}
            onPointerLeave={() => setHover(null)}
            onDoubleClick={(e) => {
              const { x, y } = local(e);
              const hit = pointAt(x, y);
              if (hit) void run({ type: "removeAutomationPoints", target, ids: [hit.id] });
            }}
            onKeyDown={onKey}
            onContextMenu={onContext}
          />
        </div>
      </div>
      {menu && <Menu at={{ x: menu.x, y: menu.y }} entries={menu.entries} onClose={() => setMenu(null)} />}
    </div>
  );
}
