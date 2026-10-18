import { memo, useEffect, useRef, useState, type PointerEvent, type ReactNode } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { ChannelMeter } from "../../bindings/ChannelMeter";
import type { ChannelRef } from "../../bindings/ChannelRef";
import type { InsertSlot } from "../../bindings/InsertSlot";
import type { MixerStatus } from "../../bindings/MixerStatus";
import type { PanLaw } from "../../bindings/PanLaw";
import type { Processor } from "../../bindings/Processor";
import type { Project } from "../../bindings/Project";
import type { StripPatch } from "../../bindings/StripPatch";
import type { StripRouting } from "../../bindings/StripRouting";
import type { TrackKind } from "../../bindings/TrackKind";
import { IconButton, Toggle } from "../../components/Button";
import { NumberField } from "../../components/Fields";
import { Icon } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { dbToFaderPos, faderPosToDb, formatDb, formatPan, MIN_DB } from "../../lib/format";
import { ipc } from "../../lib/ipc";
import { engineAvailable, useApp } from "../../state/app";
import { notify, notifyError } from "../../state/notifications";
import { useProject } from "../../state/project";
import { useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";
import { pendingControl } from "../topbar/menuActions";
import { EffectEditor } from "../effects/EffectEditor";
import { effectMenuEntries } from "../effects/effectMenu";
import { EqThumbnail } from "../effects/EqThumbnail";
import { cachedCatalog, newSlot } from "../../lib/effects";
import { effectInfo } from "../../lib/effects";
import type { AutomationMode } from "../../bindings/AutomationMode";
import type { AutomationParam } from "../../bindings/AutomationParam";
import type { TouchPhase } from "../../bindings/TouchPhase";
import { MODES, automatedValue, channelAutomation, sameParam, writesMode } from "../../lib/automation";
import { isRolling } from "../../state/transport";

/** Live automation capture for one control: `touch` sends a gesture to the
 * core and resolves to whether it was written as automation. */
type Touch = (phase: TouchPhase, value: number) => Promise<boolean>;

/** The value a control shows: its automated value while playing, else its own. */
function useAutomated(project: Project, channel: ChannelRef, param: AutomationParam, value: number): number {
  const auto = channelAutomation(project, channel);
  const automated =
    !!auto &&
    auto.mode !== "off" &&
    (auto.lanes.some((l) => sameParam(l.param, param) && l.points.length > 0) ||
      (channel.type === "track" &&
        !!project.tracks.find((t) => t.id === channel.id)?.regions.some((r) => r.automation.some((l) => sameParam(l.param, param) && l.points.length))));
  const tick = useTransport((s) => (automated && isRolling(s.snapshot) && s.snapshot ? Math.round(s.snapshot.ticks / 24) * 24 : null));
  if (tick === null) return value;
  return automatedValue(project, channel, param, tick) ?? value;
}

function Fader({
  value,
  onCommit,
  label,
  disabled,
  touch,
}: {
  value: number;
  onCommit: (db: number) => void;
  label: string;
  disabled?: boolean;
  /** Set while the channel writes automation and the transport plays. */
  touch?: Touch;
}) {
  const [preview, setPreview] = useState<number | null>(null);
  const drag = useRef<{ y: number; pos: number; height: number; touching: boolean } | null>(null);
  const db = preview ?? value;
  const pos = dbToFaderPos(db);

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled || e.button !== 0) return;
    if (e.altKey) return onCommit(0);
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = {
      y: e.clientY,
      pos,
      height: Math.max(40, e.currentTarget.getBoundingClientRect().height),
      touching: !!touch,
    };
    if (touch) void touch("begin", db);
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const sensitivity = e.shiftKey ? 0.2 : 1;
    const p = Math.min(
      1,
      Math.max(
        0,
        drag.current.pos +
          ((drag.current.y - e.clientY) / drag.current.height) * sensitivity,
      ),
    );
    const next = Math.round(faderPosToDb(p) * 10) / 10;
    setPreview(next);
    if (drag.current.touching && touch) void touch("move", Math.max(MIN_DB, next));
  };
  const onUp = () => {
    const d = drag.current;
    drag.current = null;
    const final = preview;
    setPreview(null);
    if (!d) return;
    if (d.touching && touch) {
      // Written as automation while playing; otherwise an ordinary edit.
      void touch("end", Math.max(MIN_DB, final ?? value)).then((captured) => {
        if (!captured && final !== null && final !== value) onCommit(Math.max(MIN_DB, final));
      });
      return;
    }
    if (final !== null && final !== value) onCommit(Math.max(MIN_DB, final));
  };

  return (
    <div
      className={`fader${disabled ? " is-disabled" : ""}`}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={MIN_DB}
      aria-valuemax={6}
      aria-valuenow={db}
      aria-valuetext={`${formatDb(db)} dB`}
      title={`${label}: ${formatDb(db)} dB — drag (Shift for fine), Alt-click or double-click for 0 dB`}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
      onDoubleClick={() => !disabled && onCommit(0)}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 0.1 : 1;
        if (e.key === "ArrowUp")
          onCommit(Math.min(6, Math.round((db + step) * 10) / 10));
        else if (e.key === "ArrowDown")
          onCommit(Math.max(MIN_DB, Math.round((db - step) * 10) / 10));
        else return;
        e.preventDefault();
      }}
    >
      <div className="fader__track" />
      <div
        className="fader__unity"
        style={{ bottom: `${dbToFaderPos(0) * 100}%` }}
      />
      <div
        className="fader__cap"
        style={{ bottom: `calc(${pos * 100}% - 14px)` }}
      />
    </div>
  );
}

function PanKnob({
  value,
  onCommit,
  disabled,
  touch,
}: {
  value: number;
  onCommit: (pan: number) => void;
  disabled?: boolean;
  touch?: Touch;
}) {
  const [preview, setPreview] = useState<number | null>(null);
  const drag = useRef<{ y: number; v: number } | null>(null);
  const v = preview ?? value;
  const angle = v * 135;
  return (
    <div
      className={`knob${disabled ? " is-disabled" : ""}`}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label="Pan"
      aria-valuemin={-1}
      aria-valuemax={1}
      aria-valuenow={v}
      aria-valuetext={formatPan(v)}
      title={`Pan ${formatPan(v)} — drag vertically, double-click to centre`}
      onPointerDown={(e) => {
        if (disabled) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { y: e.clientY, v: value };
        if (touch) void touch("begin", value);
      }}
      onPointerMove={(e) => {
        if (!drag.current) return;
        const next = Math.min(
          1,
          Math.max(-1, drag.current.v + (drag.current.y - e.clientY) / 100),
        );
        setPreview(Math.round(next * 64) / 64);
        if (touch) void touch("move", Math.round(next * 64) / 64);
      }}
      onPointerUp={() => {
        const had = drag.current !== null;
        const final = preview;
        drag.current = null;
        setPreview(null);
        if (!had) return;
        if (touch) {
          void touch("end", final ?? value).then((captured) => {
            if (!captured && final !== null && final !== value) onCommit(final);
          });
          return;
        }
        if (final !== null && final !== value) onCommit(final);
      }}
      onDoubleClick={() => !disabled && onCommit(0)}
      onKeyDown={(e) => {
        if (e.key === "ArrowUp" || e.key === "ArrowRight")
          onCommit(Math.min(1, Math.round((v + 1 / 32) * 64) / 64));
        else if (e.key === "ArrowDown" || e.key === "ArrowLeft")
          onCommit(Math.max(-1, Math.round((v - 1 / 32) * 64) / 64));
        else return;
        e.preventDefault();
      }}
    >
      <svg viewBox="0 0 32 32" width="30" height="30" aria-hidden>
        <path d="M6.7 25.3A13 13 0 1 1 25.3 25.3" className="knob__arc" />
        <circle cx="16" cy="16" r="9" className="knob__body" />
        <line
          x1="16"
          y1="16"
          x2="16"
          y2="8.5"
          className="knob__pointer"
          transform={`rotate(${angle} 16 16)`}
        />
      </svg>
      <span className="knob__value">{formatPan(v)}</span>
    </div>
  );
}


const FADER_SCALE = [6, 0, -6, -12, -24, -40, -60];
const HOLD_MS = 1500;

type ChannelKey = string;
const keyOf = (c: ChannelRef): ChannelKey => (c.type === "master" ? "master" : `${c.type}:${c.id}`);

/** Meter DOM nodes, animated from engine events without React renders. */
interface MeterNodes {
  peak: HTMLDivElement | null;
  rms: HTMLDivElement | null;
  hold: HTMLDivElement | null;
  clip: HTMLButtonElement | null;
  readout: HTMLSpanElement | null;
}
type MeterRegistry = Map<ChannelKey, MeterNodes>;

const toPos = (v: number) => (v > 0 ? dbToFaderPos(Math.max(MIN_DB, 20 * Math.log10(v))) : 0);

/** Drives every meter from `audio://meters` (real post-fader levels). */
function useEngineMeters(registry: MeterRegistry, project: Project | null) {
  const projectRef = useRef(project);
  useEffect(() => {
    projectRef.current = project;
  }, [project]);
  useEffect(() => {
    const holds = new Map<ChannelKey, { v: number; at: number }>();
    const show = (key: ChannelKey, m: ChannelMeter | undefined) => {
      const nodes = registry.get(key);
      if (!nodes) return;
      const peak = m ? Math.max(m.peak[0], m.peak[1]) : 0;
      const rms = m ? Math.max(m.rms[0], m.rms[1]) : 0;
      const now = performance.now();
      const h = holds.get(key);
      const hold = !h || peak >= h.v || now - h.at > HOLD_MS ? { v: peak, at: now } : h;
      holds.set(key, hold);
      if (nodes.peak) nodes.peak.style.transform = `scaleY(${toPos(peak).toFixed(3)})`;
      if (nodes.rms) nodes.rms.style.transform = `scaleY(${toPos(rms).toFixed(3)})`;
      if (nodes.hold) nodes.hold.style.bottom = `${(toPos(hold.v) * 100).toFixed(1)}%`;
      if (nodes.clip) nodes.clip.classList.toggle("is-on", !!m?.clip);
      if (nodes.readout) nodes.readout.textContent = hold.v > 0 ? formatDb(Math.max(MIN_DB, 20 * Math.log10(hold.v))) : "−∞";
    };
    return useTransport.subscribe((s, prev) => {
      if (s.meters === prev.meters || !s.meters) return;
      const p = projectRef.current;
      if (!p) return;
      p.tracks.forEach((t, i) => show(`track:${t.id}`, s.meters!.tracks[i]));
      p.mixer.auxes.forEach((a, i) => show(`aux:${a.id}`, s.meters!.auxes[i]));
      show("master", s.meters.master);
    });
  }, [registry]);
}

/** Whether routing `from` into aux `to` is accepted (mirror of `can_route`). */
function canRoute(project: Project, from: ChannelRef, to: string): boolean {
  if (from.type === "master") return false;
  if (from.type === "aux" && from.id === to) return false;
  // `to` must not reach `from` (a cycle). Only auxes have inputs.
  if (from.type !== "aux") return true;
  const next = (id: string): string[] => {
    const a = project.mixer.auxes.find((x) => x.id === id);
    if (!a) return [];
    const out = a.strip.output.type === "aux" ? [a.strip.output.id] : [];
    return [...out, ...a.strip.sends.map((s) => s.target)];
  };
  const seen = new Set<string>();
  const stack = [to];
  while (stack.length) {
    const id = stack.pop()!;
    if (id === from.id) return false;
    if (seen.has(id)) continue;
    seen.add(id);
    stack.push(...next(id));
  }
  return true;
}

const PROCESSORS: { label: string; make: () => Processor }[] = [
  { label: "Trim", make: () => ({ type: "trim", gainDb: 0 }) },
  { label: "Utility (polarity / mono / swap)", make: () => ({ type: "utility", invertLeft: false, invertRight: false, swap: false, mono: false }) },
  { label: "Filter (high-pass)", make: () => ({ type: "filter", mode: "highPass", cutoffHz: 80, q: 0.707 }) },
  { label: "Filter (low-pass)", make: () => ({ type: "filter", mode: "lowPass", cutoffHz: 12000, q: 0.707 }) },
  { label: "Filter (band-pass)", make: () => ({ type: "filter", mode: "bandPass", cutoffHz: 1000, q: 1 }) },
  { label: "Filter (notch)", make: () => ({ type: "filter", mode: "notch", cutoffHz: 1000, q: 4 }) },
  { label: "Limiter (look-ahead)", make: () => ({ type: "limiter", ceilingDb: -1, lookaheadMs: 5, releaseMs: 80 }) },
];

function processorLabel(p: Processor): string {
  switch (p.type) {
    case "trim":
      return `Trim ${formatDb(p.gainDb)}`;
    case "utility":
      return ["Util", p.invertLeft || p.invertRight ? "Ø" : "", p.mono ? "M" : "", p.swap ? "⇄" : ""].filter(Boolean).join(" ");
    case "filter":
      return `${{ highPass: "HP", lowPass: "LP", bandPass: "BP", notch: "Notch" }[p.mode]} ${p.cutoffHz >= 1000 ? `${(p.cutoffHz / 1000).toFixed(1)}k` : Math.round(p.cutoffHz)}`;
    case "limiter":
      return `Lim ${formatDb(p.ceilingDb)}`;
    case "effect":
      return effectInfo(p.kind)?.short ?? p.kind;
  }
}

/** Parameter editor for one insert (popover). */
function InsertEditor({ channel, slot, index, count, onClose, at }: { channel: ChannelRef; slot: InsertSlot; index: number; count: number; onClose: () => void; at: { x: number; y: number } }) {
  const apply = useProject((s) => s.apply);
  const set = (processor: Processor, bypass = slot.bypass) => void apply({ type: "setInsert", channel, id: slot.id, bypass, processor });
  const p = slot.processor;
  const fields: ReactNode = (() => {
    switch (p.type) {
      case "trim":
        return <NumberField label="Gain" value={p.gainDb} min={-48} max={24} step={0.5} precision={1} suffix="dB" onCommit={(gainDb) => set({ ...p, gainDb })} />;
      case "utility":
        return (
          <>
            <Toggle label="Invert left" checked={p.invertLeft} onChange={(invertLeft) => set({ ...p, invertLeft })} />
            <Toggle label="Invert right" checked={p.invertRight} onChange={(invertRight) => set({ ...p, invertRight })} />
            <Toggle label="Mono" checked={p.mono} onChange={(mono) => set({ ...p, mono })} />
            <Toggle label="Swap L/R" checked={p.swap} onChange={(swap) => set({ ...p, swap })} />
          </>
        );
      case "filter":
        return (
          <>
            <select className="pr__select" aria-label="Filter mode" value={p.mode} onChange={(e) => set({ ...p, mode: e.target.value as typeof p.mode })}>
              <option value="highPass">High-pass</option>
              <option value="lowPass">Low-pass</option>
              <option value="bandPass">Band-pass</option>
              <option value="notch">Notch</option>
            </select>
            <NumberField label="Cutoff" value={p.cutoffHz} min={20} max={20000} step={10} suffix="Hz" onCommit={(cutoffHz) => set({ ...p, cutoffHz })} />
            <NumberField label="Q" value={p.q} min={0.3} max={10} step={0.05} precision={2} onCommit={(q) => set({ ...p, q })} />
          </>
        );
      case "limiter":
        return (
          <>
            <NumberField label="Ceiling" value={p.ceilingDb} min={-24} max={0} step={0.1} precision={1} suffix="dB" onCommit={(ceilingDb) => set({ ...p, ceilingDb })} />
            <NumberField label="Look-ahead (latency)" value={p.lookaheadMs} min={0} max={20} step={0.5} precision={1} suffix="ms" onCommit={(lookaheadMs) => set({ ...p, lookaheadMs })} />
            <NumberField label="Release" value={p.releaseMs} min={1} max={1000} step={5} suffix="ms" onCommit={(releaseMs) => set({ ...p, releaseMs })} />
          </>
        );
    }
  })();
  const entries: MenuEntry[] = [
    { heading: PROCESSORS.find((x) => x.make().type === p.type)?.label ?? p.type },
    { key: "fields", custom: <div className="insert-editor" onKeyDown={(e) => e.stopPropagation()}>{fields}</div> },
    "separator",
    { id: "bypass", label: slot.bypass ? "Enable" : "Bypass", onSelect: () => set(p, !slot.bypass) },
    { id: "up", label: "Move up", disabled: index === 0, onSelect: () => void apply({ type: "moveInsert", channel, id: slot.id, toIndex: index - 1 }) },
    { id: "down", label: "Move down", disabled: index === count - 1, onSelect: () => void apply({ type: "moveInsert", channel, id: slot.id, toIndex: index + 1 }) },
    { id: "rm", label: "Remove", danger: true, onSelect: () => void apply({ type: "removeInsert", channel, id: slot.id }) },
  ];
  return <Menu at={at} entries={entries} onClose={onClose} />;
}

interface StripModel {
  channel: ChannelRef;
  name: string;
  color: string;
  volumeDb: number;
  pan: number;
  muted: boolean;
  soloed: boolean;
  routing: StripRouting | null;
  inserts: InsertSlot[];
  /** MIDI tracks have no audio channel in the engine. */
  audio: boolean;
  kindLabel: string;
}

function stripsOf(project: Project): { tracks: StripModel[]; auxes: StripModel[]; master: StripModel } {
  const tracks = project.tracks.map((t) => ({
    channel: { type: "track", id: t.id } as ChannelRef,
    name: t.name,
    color: t.color,
    volumeDb: t.volumeDb,
    pan: t.pan,
    muted: t.muted,
    soloed: t.soloed,
    routing: t.strip,
    inserts: t.strip.inserts,
    audio: t.kind !== "midi",
    kindLabel: t.kind === "audio" ? "Audio" : t.kind === "instrument" ? "Test Tone" : "MIDI Out",
  }));
  const auxes = project.mixer.auxes.map((a) => ({
    channel: { type: "aux", id: a.id } as ChannelRef,
    name: a.name,
    color: a.color,
    volumeDb: a.volumeDb,
    pan: a.pan,
    muted: a.muted,
    soloed: a.soloed,
    routing: a.strip,
    inserts: a.strip.inserts,
    audio: true,
    kindLabel: "Aux",
  }));
  const m = project.mixer.master;
  const master: StripModel = {
    channel: { type: "master" },
    name: "Stereo Out",
    color: "#6b6b6e",
    volumeDb: m.volumeDb,
    pan: 0,
    muted: m.muted,
    soloed: false,
    routing: null,
    inserts: m.inserts,
    audio: true,
    kindLabel: "Master",
  };
  return { tracks, auxes, master };
}

const ChannelStrip = memo(function ChannelStrip({
  strip,
  project,
  selected,
  registry,
  onSelect,
}: {
  strip: StripModel;
  project: Project;
  selected: boolean;
  registry: MeterRegistry;
  onSelect: (s: StripModel) => void;
}) {
  const apply = useProject((s) => s.apply);
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; entries: MenuEntry[] } | null>(null);
  const [editing, setEditing] = useState<{ index: number; at: { x: number; y: number } } | null>(null);
  /** Native effect insert open in the effect editor. */
  const [fxEditing, setFxEditing] = useState<string | null>(null);
  const c = strip.channel;
  const key = keyOf(c);
  const patch = (p: StripPatch) => void apply({ type: "setStrip", channel: c, patch: p });
  const nodes = (): MeterNodes => {
    let n = registry.get(key);
    if (!n) {
      n = { peak: null, rms: null, hold: null, clip: null, readout: null };
      registry.set(key, n);
    }
    return n;
  };
  const at = (e: { clientX: number; clientY: number }) => ({ x: e.clientX, y: e.clientY });
  const auxName = (id: string) => project.mixer.auxes.find((a) => a.id === id)?.name ?? "?";
  const routing = strip.routing;
  const output = routing?.output;
  const disabledAudio = !strip.audio;
  const groups = project.mixer.groups;
  const group = routing?.group ? groups.find((g) => g.id === routing.group) : undefined;
  const auto = channelAutomation(project, c);
  const mode: AutomationMode = auto?.mode ?? "read";
  const rolling = useTransport((s) => isRolling(s.snapshot));
  const capture = rolling && writesMode(mode) && !disabledAudio;
  const touchOf = (param: AutomationParam): Touch | undefined =>
    capture ? (phase, value) => ipc.automation.touch(c, param, value, phase).catch(() => false) : undefined;
  /** A discrete control (mute, a typed send level): one begin/end gesture. */
  const tap = async (param: AutomationParam, value: number, fallback: () => void) => {
    const t = touchOf(param);
    if (t && (await t("begin", value))) {
      await t("end", value);
      return;
    }
    fallback();
  };
  const shownVolume = useAutomated(project, c, { type: "volume" }, strip.volumeDb);
  const shownPan = useAutomated(project, c, { type: "pan" }, strip.pan);
  const shownMute = useAutomated(project, c, { type: "mute" }, strip.muted ? 1 : 0) >= 0.5;

  const outputMenu = (e: React.MouseEvent) =>
    setMenu({
      at: at(e),
      entries: [
        { id: "master", label: "Stereo Out", icon: output?.type === "master" ? "check" : undefined, onSelect: () => patch({ output: { type: "master" } }) },
        ...project.mixer.auxes.map((a) => ({
          id: a.id,
          label: `Bus → ${a.name}`,
          icon: output?.type === "aux" && output.id === a.id ? ("check" as const) : undefined,
          disabled: !canRoute(project, c, a.id),
          onSelect: () => patch({ output: { type: "aux", id: a.id } }),
        })),
        "separator",
        {
          id: "new",
          label: "New Aux Bus…",
          onSelect: async () => {
            const out = await apply({ type: "addAux", sources: [] });
            const id = out?.createdAuxes[0];
            if (id) patch({ output: { type: "aux", id } });
          },
        },
      ],
    });
  const sendMenu = (e: React.MouseEvent) =>
    setMenu({
      at: at(e),
      entries: [
        { heading: "Add send to" },
        ...project.mixer.auxes
          .filter((a) => !routing?.sends.some((s) => s.target === a.id))
          .map((a) => ({
            id: a.id,
            label: a.name,
            disabled: !canRoute(project, c, a.id),
            onSelect: () => void apply({ type: "setSend", channel: c, target: a.id, send: { target: a.id, levelDb: 0, preFader: false, muted: false } }),
          })),
        { id: "new", label: "New Aux…", onSelect: () => void apply({ type: "addAux", sources: [c] }) },
      ],
    });
  const groupMenu = (e: React.MouseEvent) =>
    setMenu({
      at: at(e),
      entries: [
        { id: "none", label: "No group", icon: !group ? "check" : undefined, onSelect: () => void apply({ type: "setGroupMembership", channels: [c] }) },
        ...groups.map((g) => ({
          id: g.id,
          label: g.name,
          icon: g.id === group?.id ? ("check" as const) : undefined,
          onSelect: () => void apply({ type: "setGroupMembership", channels: [c], group: g.id }),
        })),
        { id: "new", label: "New Group (links volume, mute, solo)", onSelect: () => void apply({ type: "addGroup", members: [c] }) },
      ],
    });
  /** The strip's first parametric equaliser, shown in the EQ slot. */
  const stripEq = strip.inserts.find((s) => s.processor.type === "effect" && s.processor.kind === "parametricEq");
  const openEq = () => {
    if (stripEq) return setFxEditing(stripEq.id);
    const info = cachedCatalog().find((e) => e.kind === "parametricEq");
    if (!info || strip.inserts.length >= 8) return;
    void apply({ type: "addInsert", channel: c, processor: { type: "effect", ...newSlot(info) }, index: 0 }).then((r) => {
      const id = r?.createdInserts[0];
      if (id) setFxEditing(id);
    });
  };
  const insertMenu = (e: React.MouseEvent) =>
    setMenu({
      at: at(e),
      entries: [
        { heading: "Native processors" },
        ...PROCESSORS.map((p) => ({ id: p.label, label: p.label, onSelect: () => void apply({ type: "addInsert", channel: c, processor: p.make() }) })),
        "separator",
        ...effectMenuEntries((slot) =>
          void apply({ type: "addInsert", channel: c, processor: { type: "effect", ...slot } }).then((r) => {
            const id = r?.createdInserts[0];
            if (id) setFxEditing(id);
          }),
        ),
        "separator",
        { id: "plug", label: "Third-party plug-ins: not available yet", disabled: true, onSelect: () => undefined },
      ],
    });

  return (
    <div
      className={`strip${selected ? " is-selected" : ""}${c.type === "master" ? " strip--master" : ""}${c.type === "aux" ? " strip--aux" : ""}${disabledAudio ? " is-midi" : ""}`}
      style={{ ["--track-color" as string]: strip.color }}
      onPointerDown={() => onSelect(strip)}
      data-channel={key}
    >
      <button className="slot strip__setting" type="button" onClick={() => pendingControl("Ajuste de canal")}>{c.type === "track" ? strip.name : "Ajuste"}</button>
      <button
        className="strip__eq"
        type="button"
        disabled={disabledAudio}
        title={stripEq ? "Abrir el ecualizador del canal (clic derecho: añadir efecto)" : "Añadir y abrir un ecualizador paramétrico (clic derecho: añadir efecto)"}
        onClick={openEq}
        onContextMenu={(e) => {
          e.preventDefault();
          insertMenu(e);
        }}
      >
        {stripEq && stripEq.processor.type === "effect" ? <EqThumbnail params={stripEq.processor.params} bypass={stripEq.bypass} /> : "Ecualizador"}
      </button>
      <button className="slot strip__midi-fx" type="button" onClick={() => pendingControl("Efectos MIDI")}>Efectos MIDI</button>
      <div className="slot slot--input" title={strip.kindLabel === "Test Tone" ? "Built-in test instrument (sine + harmonic, ADSR); not a production synthesizer" : strip.kindLabel}>
        {strip.kindLabel === "Test Tone" ? "Instrumento" : strip.kindLabel === "Master" ? <Icon name="link" size={13} /> : strip.kindLabel}
      </div>
      <div className="strip__fx">
        {strip.inserts.map((slot, i) => (
          <button
            key={slot.id}
            type="button"
            className={`slot slot--fx${slot.bypass ? " is-bypassed" : ""}`}
            title={`${processorLabel(slot.processor)}${slot.bypass ? " (bypassed)" : ""} — click to edit`}
            disabled={disabledAudio}
            onClick={(e) => (slot.processor.type === "effect" ? setFxEditing(slot.id) : setEditing({ index: i, at: at(e) }))}
            onContextMenu={(e) => {
              e.preventDefault();
              setEditing({ index: i, at: at(e) });
            }}
          >
            {processorLabel(slot.processor)}
          </button>
        ))}
        {strip.inserts.length < 8 && (
          <button type="button" className="slot slot--add" title="Add insert" disabled={disabledAudio} onClick={insertMenu}>
            Efec. audio
          </button>
        )}
      </div>
      {routing && (
        <div className="strip__sends">
          {routing.sends.map((s) => (
            <div key={s.target} className={`send${s.muted ? " is-muted" : ""}`}>
              <button
                type="button"
                className="send__name"
                title={`${s.preFader ? "Pre" : "Post"}-fader send to ${auxName(s.target)} — click for options`}
                onClick={(e) =>
                  setMenu({
                    at: at(e),
                    entries: [
                      { heading: `Send → ${auxName(s.target)}` },
                      { id: "pre", label: s.preFader ? "Post-fader" : "Pre-fader", onSelect: () => void apply({ type: "setSend", channel: c, target: s.target, send: { ...s, preFader: !s.preFader } }) },
                      { id: "mute", label: s.muted ? "Unmute send" : "Mute send", onSelect: () => void apply({ type: "setSend", channel: c, target: s.target, send: { ...s, muted: !s.muted } }) },
                      { id: "rm", label: "Remove send", danger: true, onSelect: () => void apply({ type: "setSend", channel: c, target: s.target }) },
                    ],
                  })
                }
              >
                {s.preFader ? "▸" : ""}
                {auxName(s.target)}
              </button>
              <NumberField
                className="send__level"
                label={`Send level to ${auxName(s.target)}`}
                value={s.levelDb}
                min={-96}
                max={6}
                step={0.5}
                precision={1}
                onCommit={(levelDb) =>
                  void tap({ type: "sendLevel", aux: s.target }, levelDb, () => void apply({ type: "setSend", channel: c, target: s.target, send: { ...s, levelDb } }))
                }
              />
            </div>
          ))}
          {routing.sends.length < 8 && (
            <button type="button" className="slot slot--add" title="Add send" disabled={disabledAudio} onClick={sendMenu}>
              Envíos
            </button>
          )}
        </div>
      )}
      {routing ? (
        <button type="button" className="slot slot--output" title="Output routing" onClick={outputMenu}>
          {output?.type === "aux" ? auxName(output.id) : "Sal estéreo"}
        </button>
      ) : (
        <div className="slot slot--output" title="Device output">
          Output 1–2
        </div>
      )}
      {routing ? (
        <button type="button" className="slot slot--group" title={group ? `Group ${group.name}` : "Group: none"} onClick={groupMenu} style={group ? { background: group.color, color: "#fff" } : undefined}>
          {group?.name ?? "Grupo"}
        </button>
      ) : (
        <div className="slot slot--group" aria-hidden>Grupo</div>
      )}
      <select
        className={`slot strip__read strip__auto is-${mode}`}
        aria-label={`${strip.name} automation mode`}
        title={disabledAudio ? "External MIDI tracks have no audio channel to automate" : "Automation mode: Read plays lanes; Touch, Latch and Write record control moves while playing"}
        disabled={disabledAudio}
        value={mode}
        onChange={(e) => void apply({ type: "setAutomationMode", channel: c, mode: e.target.value as AutomationMode })}
      >
        {MODES.map((m) => (
          <option key={m.mode} value={m.mode} title={m.title}>
            {m.label}
          </option>
        ))}
      </select>
      {routing && (
        <div className="strip__flags">
          <button
            type="button"
            className="strip__flag"
            title={`Channel format: ${routing.format} — click to cycle Auto / Mono / Stereo`}
            disabled={disabledAudio}
            onClick={() => patch({ format: routing.format === "auto" ? "mono" : routing.format === "mono" ? "stereo" : "auto" })}
          >
            {routing.format === "auto" ? "A" : routing.format === "mono" ? "●" : "●●"}
          </button>
          <button
            type="button"
            className={`strip__flag${routing.soloSafe ? " is-on" : ""}`}
            aria-pressed={routing.soloSafe}
            title="Solo safe: stays audible when other channels are soloed"
            onClick={() => patch({ soloSafe: !routing.soloSafe })}
          >
            <Icon name="lock" size={10} />
          </button>
        </div>
      )}
      <PanKnob
        value={shownPan}
        disabled={disabledAudio || c.type === "master"}
        touch={c.type === "master" ? undefined : touchOf({ type: "pan" })}
        onCommit={(pan) => patch({ pan })}
      />
      <div className="strip__db">
        <span className="strip__db-box" title="Fader level">
          {formatDb(shownVolume)}
        </span>
        <span className="strip__db-box strip__db-box--peak" title="Peak hold (1.5 s), measured on this channel's real signal" ref={(el) => void (nodes().readout = el)}>
          −∞
        </span>
      </div>
      <div className="strip__fader-row">
        <div className="fader-scale" aria-hidden>
          {FADER_SCALE.map((db) => (
            <span key={db} style={{ bottom: `${dbToFaderPos(db) * 100}%` }}>
              {db > 0 ? `+${db}` : db}
            </span>
          ))}
        </div>
        <div className="meter" title="Post-fader peak (bright), RMS (dim) and peak hold, from the audio engine">
          <button
            type="button"
            className="meter__clip"
            title="Clip: a sample exceeded 0 dBFS. Click to reset all clip indicators."
            ref={(el) => void (nodes().clip = el)}
            onClick={() => void ipc.mixer.resetClips().catch(notifyError)}
          />
          <div className="meter__bar" ref={(el) => void (nodes().peak = el)} />
          <div className="meter__rms" ref={(el) => void (nodes().rms = el)} />
          <div className="meter__hold" ref={(el) => void (nodes().hold = el)} />
        </div>
        <Fader value={shownVolume} label={`${strip.name} volume`} disabled={disabledAudio} touch={touchOf({ type: "volume" })} onCommit={(volumeDb) => patch({ volumeDb })} />
      </div>
      <div className="strip__ms">
        <button
          type="button"
          className={`ms-btn ms-btn--mute${shownMute ? " is-on" : ""}`}
          aria-pressed={shownMute}
          onClick={() => void tap({ type: "mute" }, shownMute ? 0 : 1, () => patch({ muted: !strip.muted }))}
        >
          M
        </button>
        {c.type !== "master" && (
          <button type="button" className={`ms-btn ms-btn--solo${strip.soloed ? " is-on" : ""}`} aria-pressed={strip.soloed} onClick={() => patch({ soloed: !strip.soloed })}>
            S
          </button>
        )}
      </div>
      <div className="strip__name" title={disabledAudio ? `${strip.name} — MIDI to an external port; no audio channel` : strip.name}>
        <span>{strip.name}</span>
      </div>
      {menu && <Menu at={menu.at} entries={menu.entries} onClose={() => setMenu(null)} />}
      {editing && strip.inserts[editing.index] && (
        <InsertEditor channel={c} slot={strip.inserts[editing.index]!} index={editing.index} count={strip.inserts.length} at={editing.at} onClose={() => setEditing(null)} />
      )}
      {fxEditing && (
        <EffectEditor channel={c} insertId={fxEditing} onClose={() => setFxEditing(null)} />
      )}
    </div>
  );
});

/** The inspector uses the same native channel controls as the full mixer. */
export function InspectorChannels({ trackId }: { trackId?: string }) {
  const project = useProject(s => s.view?.project);
  const [registry] = useState<MeterRegistry>(() => new Map());
  useEngineMeters(registry, project ?? null);
  if (!project) return null;
  const { tracks, master } = stripsOf(project);
  const focused = tracks.find(t => t.channel.type === "track" && t.channel.id === trackId) ?? tracks[0];
  return <div className="inspector-channels">
    {[...(focused ? [focused] : []), master].map(s => <ChannelStrip key={keyOf(s.channel)} strip={s} project={project} selected={false} registry={registry} onSelect={() => undefined} />)}
  </div>;
}

const KIND_FILTERS: { kind: TrackKind; label: string }[] = [
  { kind: "audio", label: "Audio" },
  { kind: "instrument", label: "Inst" },
  { kind: "midi", label: "MIDI" },
];

const PAN_LAWS: { law: PanLaw; label: string }[] = [
  { law: "constantPower", label: "−3 dB" },
  { law: "constantPowerCompensated", label: "0 dB comp." },
  { law: "linear", label: "−6 dB linear" },
];

export function Mixer() {
  const project = useProject((s) => s.view?.project) ?? null;
  const apply = useProject((s) => s.apply);
  const engine = useApp((s) => s.engine);
  const selection = useUi((s) => s.selection);
  const [registry] = useState<MeterRegistry>(() => new Map());
  const [hidden, setHidden] = useState<TrackKind[]>([]);
  const [latency, setLatency] = useState<MixerStatus | null>(null);
  const [exporting, setExporting] = useState(false);
  const [scope, setScope] = useState("Pistas");
  useEngineMeters(registry, project);
  useEffect(() => {
    if (!project) return;
    void ipc.mixer
      .status()
      .then(setLatency)
      .catch(() => undefined);
  }, [project]);
  if (!project) return null;
  const { tracks, auxes, master } = stripsOf(project);
  const onSelect = (s: StripModel) => {
    if (s.channel.type === "track") useUi.getState().selectTracks([s.channel.id], s.channel.id);
  };
  const folders = project.mixer.folders;
  const visible = tracks.filter((s, i) => !hidden.includes(project.tracks[i]!.kind) && (scope !== "Sencillo" || (s.channel.type === "track" && selection.trackIds.includes(s.channel.id))));
  const folderOf = (s: StripModel) => (s.channel.type === "track" ? project.tracks.find((t) => t.id === (s.channel as { id: string }).id)?.folder ?? null : null);
  const loose = visible.filter((s) => !folderOf(s));
  // Every strip's insert area (and its label) is as tall as the fullest one,
  // so rows stay aligned however many effects a channel has.
  const fxRows = Math.max(1, ...[...tracks, ...auxes, master].map((s) => s.inserts.length + (s.inserts.length < 8 ? 1 : 0)));
  const fxStyle = { "--fx-height": `${34 + (fxRows - 1) * 21}px`, "--fx-extra": `${(fxRows - 1) * 21}px` } as React.CSSProperties;
  const exportStems = async () => {
    const dir = await openDialog({ directory: true, title: "Export stems to folder" });
    if (typeof dir !== "string") return;
    setExporting(true);
    try {
      const report = await ipc.media.exportStems(dir);
      notify("success", `Exported ${report.stems.length} stems (master peak ${formatDb(20 * Math.log10(Math.max(1e-5, Math.max(...report.master.peak))))} dBFS).`);
    } catch (e) {
      notifyError(e);
    } finally {
      setExporting(false);
    }
  };
  const selectedTrackIds = selection.trackIds;
  const strip = (s: StripModel) => (
    <ChannelStrip
      key={keyOf(s.channel)}
      strip={s}
      project={project}
      selected={s.channel.type === "track" && selection.trackIds.includes(s.channel.id)}
      registry={registry}
      onSelect={onSelect}
    />
  );

  return (
    <div className="mixer">
      <div className="mixer__bar">
        <button className="arrangement-menu" onClick={() => pendingControl("Edición del mezclador")}>Edición <Icon name="chevronDown" size={11} /></button>
        <details className="mixer-options"><summary className="arrangement-menu">Opciones <Icon name="chevronDown" size={11} /></summary><div className="mixer-options__content">
        <span className="mixer__heading">Mixer</span>
        {!engineAvailable(engine) && (
          <span className="mixer__notice" title="Routing edits the project; there is no audio signal without the engine.">
            <Icon name="info" size={12} />
            <span>No signal — audio engine offline</span>
          </span>
        )}
        {engine?.state === "running" && engine.output === "internalClock" && (
          <span className="mixer__notice" title="No audio device is open; the engine runs on its internal clock.">
            <Icon name="info" size={12} />
            <span>No audio device — meters show the engine signal</span>
          </span>
        )}
        <button type="button" className="pr__btn" onClick={() => void apply({ type: "addAux", sources: [] })}>
          + Aux
        </button>
        <button
          type="button"
          className="pr__btn"
          disabled={selectedTrackIds.length === 0}
          title="Put the selected tracks in a new mixer folder"
          onClick={() => void apply({ type: "addFolder", tracks: selectedTrackIds })}
        >
          + Folder
        </button>
        <label className="pr__check" title="Pan law for mono channels">
          Pan law
          <select className="pr__select" value={project.mixer.panLaw} onChange={(e) => void apply({ type: "setPanLaw", law: e.target.value as PanLaw })}>
            {PAN_LAWS.map((p) => (
              <option key={p.law} value={p.law}>
                {p.label}
              </option>
            ))}
          </select>
        </label>
        {latency && latency.latencyFrames > 0 && (
          <span className="hint" title="Plug-in delay compensated on every parallel path">
            PDC {latency.latencyFrames} smp ({((latency.latencyFrames / latency.sampleRate) * 1000).toFixed(1)} ms)
          </span>
        )}
        <div className="segmented segmented--blue" role="group" aria-label="Show track types">
          {KIND_FILTERS.map((f) => {
            const on = !hidden.includes(f.kind);
            return (
              <button key={f.kind} type="button" className={on ? "is-on" : ""} aria-pressed={on} onClick={() => setHidden(on ? [...hidden, f.kind] : hidden.filter((k) => k !== f.kind))}>
                {f.label}
              </button>
            );
          })}
        </div>
        <div className="mixer__bar-end">
          <button type="button" className="pr__btn" disabled={exporting || project.tracks.length === 0} onClick={() => void exportStems()} title="Render master and post-fader stems (32-bit float WAV)">
            <Icon name="exportAudio" size={12} /> {exporting ? "Exporting…" : "Export Stems"}
          </button>
        </div>
        </div></details>
        <button className="arrangement-menu" onClick={() => pendingControl("Visualización del mezclador")}>Visualización <Icon name="chevronDown" size={11} /></button>
        <span className="mixer-send-label">Envío los faders:</span>
        <button className="arrangement-menu" onClick={() => pendingControl("Envíos a faders")}><Icon name="power" size={15} /> Desactivado <Icon name="chevronDown" size={11} /></button>
        <span className="toolbar-space" />
        <div className="segmented" role="group" aria-label="Vista del mezclador">{["Sencillo", "Pistas", "Todo"].map(name => <button key={name} className={scope === name ? "is-on" : ""} aria-pressed={scope === name} onClick={() => setScope(name)}>{name}</button>)}</div>
        <span className="toolbar-space" />
        <button className="arrangement-menu" title="Componentes del canal" onClick={() => pendingControl("Componentes del canal")}><Icon name="more" size={16} /><Icon name="chevronDown" size={11} /></button>
        <button className="arrangement-menu is-blue" title="Canales verticales" onClick={() => pendingControl("Disposición de canales")}><Icon name="sliders" size={17} /></button>
      </div>
      <div className="mixer__body">
        <div className="mixer__strips" style={fxStyle}>
          <div className="mixer-labels" aria-hidden><div className="mixer-labels__stack"><span>Ajuste</span><span className="mixer-labels__eq">Ecualizador</span><span>Efectos MIDI</span><span>Entrada</span><span className="mixer-labels__fx">Efectos de audio</span><span>Envíos</span><span>Salida</span><span>Agrupar</span><span>Automatización</span><span className="mixer-labels__pan">Pan</span><span>dB</span><span className="mixer-labels__fader" /><span className="mixer-labels__footer" /></div></div>
          {project.tracks.length === 0 && auxes.length === 0 && <p className="mixer__empty">Add tracks to see their channel strips.</p>}
          {loose.map(strip)}
          {folders.map((f) => {
            const members = visible.filter((s) => folderOf(s) === f.id);
            return (
              <div key={f.id} className={`mixer-folder${f.collapsed ? " is-collapsed" : ""}`} style={{ ["--track-color" as string]: f.color }}>
                <div className="mixer-folder__head">
                  <button type="button" className="mixer-folder__toggle" aria-expanded={!f.collapsed} onClick={() => void apply({ type: "updateFolder", folder: { ...f, collapsed: !f.collapsed } })}>
                    <Icon name={f.collapsed ? "chevronRight" : "chevronDown"} size={10} />
                    {f.name} ({members.length})
                  </button>
                  <IconButton icon="close" size={10} label={`Remove folder ${f.name} (tracks are kept)`} onClick={() => void apply({ type: "removeFolder", id: f.id })} />
                </div>
                {!f.collapsed && <div className="mixer-folder__strips">{members.map(strip)}</div>}
              </div>
            );
          })}
          {auxes.length > 0 && <div className="mixer__divider" aria-hidden />}
          {auxes.map((s) => (
            <div key={keyOf(s.channel)} className="mixer__aux">
              {strip(s)}
              <IconButton icon="trash" size={10} label={`Delete ${s.name} (its sources go to the master)`} onClick={() => void apply({ type: "removeAuxes", ids: [(s.channel as { id: string }).id] })} />
            </div>
          ))}
          {(project.tracks.length > 0 || auxes.length > 0) && strip(master)}
        </div>
      </div>
    </div>
  );
}
