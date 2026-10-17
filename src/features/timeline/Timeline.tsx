import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { GridSpec } from "../../bindings/GridSpec";
import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import { Button, IconButton } from "../../components/Button";
import { Menu, type MenuEntry } from "../../components/Popover";
import {
  gridFor,
  MAX_PX_PER_QUARTER,
  MIN_PX_PER_QUARTER,
  pxToTicks,
  RULER_HEIGHT,
  ticksToPx,
  TRACK_HEIGHT,
  visibleTicks,
  zoomAround,
} from "../../lib/geometry";
import { gridBackground } from "../../lib/gridBackground";
import { inTauri } from "../../lib/ipc";
import { PPQ, ticksPerBar, ticksPerBeat } from "../../lib/music";
import { hasPrimaryModifier } from "../../lib/platform";
import { GRID_LABELS, snapTick, TempoMap } from "../../lib/tempoMap";
import { notify } from "../../state/notifications";
import { useProject } from "../../state/project";
import { isRolling, useTransport } from "../../state/transport";
import { nextTrackSelection, useUi } from "../../state/ui";
import { addTrack, duplicateSelectedTracks, deleteSelectedTracks } from "../project/editActions";
import { openDemoProject } from "../project/projectActions";
import {
  clipboardSize,
  copySelectedRegions,
  cutSelectedRegions,
  duplicateSelectedRegions,
  importAudioDialog,
  importAudioFiles,
  pasteRegions,
  setSelectedRegionsMuted,
  splitSelectedAtPlayhead,
} from "../project/regionActions";
import {
  bounceSelected,
  crossfadeWithNext,
  glueSelected,
  markersAtTransients,
  normalizeSelected,
  reverseSelected,
  rippleDeleteSelected,
  setFadeCurve,
  sliceAtTransients,
  splitMarquee,
} from "../project/editingActions";
import { AddTrackButton } from "../tracks/AddTrackMenu";
import { TrackHeader } from "../tracks/TrackHeader";
import { ArrangementMenus } from "./ArrangementMenus";
import { MeterGrid } from "./MeterGrid";
import { RegionView, type AudioDisplay } from "./RegionView";
import { Ruler } from "./Ruler";
import { useViewport } from "./useViewport";

interface RegionDrag {
  ids: string[];
  /** The region under the pointer; its start snaps to the grid. */
  primary: Region;
  startX: number;
  startY: number;
  moved: boolean;
  dTicks: number;
  dTracks: number;
  valid: boolean;
}

interface TrimDrag {
  region: Region;
  edge: "start" | "end";
  startX: number;
  start: number;
  end: number;
  moved: boolean;
}

interface SlipDrag {
  region: Region;
  startX: number;
  dTicks: number;
}

interface MarqueeDrag {
  startTick: number;
  startRow: number;
}

interface TrackDrag {
  id: string;
  startY: number;
  moved: boolean;
  /** Insert position in the track list. */
  target: number;
}

function regionTrackIndex(project: Project): Map<string, number> {
  const m = new Map<string, number>();
  project.tracks.forEach((t, i) => t.regions.forEach((r) => m.set(r.id, i)));
  return m;
}

function canMove(project: Project, ids: string[], index: Map<string, number>, dTracks: number): boolean {
  for (const id of ids) {
    const from = index.get(id);
    if (from === undefined) return false;
    const target = project.tracks[from + dTracks];
    if (!target) return false;
    const region = project.tracks[from]!.regions.find((r) => r.id === id)!;
    if ((region.content.type === "audio") !== (target.kind === "audio")) return false;
  }
  return true;
}

/** Snap spec matching the zoom-dependent grid when the user chose none. */
function autoSpec(snapTicks: number, barTicks: number, beatTicks: number): GridSpec {
  const division = snapTicks >= barTicks ? "bar" : snapTicks >= beatTicks ? "beat" : snapTicks >= PPQ / 2 ? "eighth" : "sixteenth";
  return { division, triplet: false, swing: 0 };
}

const SNAP_CHOICES: (GridSpec | null)[] = [
  null,
  ...(["bar", "beat", "quarter", "eighth", "sixteenth", "thirtySecond"] as const).map((division) => ({ division, triplet: false, swing: 0 })),
  { division: "eighth", triplet: true, swing: 0 },
  { division: "sixteenth", triplet: true, swing: 0 },
  { division: "eighth", triplet: false, swing: 50 },
  { division: "sixteenth", triplet: false, swing: 50 },
];

function specLabel(s: GridSpec | null): string {
  if (!s) return "Auto";
  return `${GRID_LABELS[s.division]}${s.triplet ? "T" : ""}${s.swing ? ` swing ${s.swing}%` : ""}`;
}

function EmptyProject() {
  return (
    <div className="timeline-empty__card">
      <h3>Start with a track</h3>
      <p>Choose the kind of track to add, or import audio files (drag them here or use Functions ▸ Import audio).</p>
      <div className="timeline-empty__actions">
        <Button icon="audio" onClick={() => void addTrack("audio")}>
          Audio
        </Button>
        <Button icon="instrument" onClick={() => void addTrack("instrument")}>
          Instrument
        </Button>
        <Button icon="midi" onClick={() => void addTrack("midi")}>
          External MIDI
        </Button>
        <Button icon="importAudio" onClick={() => void importAudioDialog()}>
          Import audio…
        </Button>
      </div>
      <button type="button" className="link-btn" onClick={() => void openDemoProject()}>
        Or explore the demo session
      </button>
    </div>
  );
}

function MarqueeView({
  marquee,
  rows,
  pxPerQuarter,
  onMenu,
}: {
  marquee: { start: number; end: number };
  rows: number[];
  pxPerQuarter: number;
  onMenu: (at: { x: number; y: number }) => void;
}) {
  if (!rows.length) return null;
  const top = Math.min(...rows) * TRACK_HEIGHT;
  const height = (Math.max(...rows) + 1) * TRACK_HEIGHT - top;
  return (
    <div
      className="marquee"
      data-testid="marquee"
      style={{ left: ticksToPx(marquee.start, pxPerQuarter), width: ticksToPx(marquee.end - marquee.start, pxPerQuarter), top, height }}
      onPointerDown={(e) => e.stopPropagation()}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onMenu({ x: e.clientX, y: e.clientY });
      }}
    />
  );
}

export function Timeline() {
  const project = useProject((s) => s.view?.project);
  const demo = useProject((s) => s.view?.origin === "demo");
  const apply = useProject((s) => s.apply);
  const pxPerQuarter = useUi((s) => s.pxPerQuarter);
  const setZoom = useUi((s) => s.setZoom);
  const cursorTicks = useUi((s) => s.cursorTicks);
  const selection = useUi((s) => s.selection);
  const headerWidth = useUi((s) => s.layout.trackHeaderWidth);
  const snapSpec = useUi((s) => s.snapSpec);
  const setSnapSpec = useUi((s) => s.setSnapSpec);
  const snapshot = useTransport((s) => s.snapshot);
  const follow = useTransport((s) => s.follow);
  const mediaStates = useTransport((s) => s.media);
  const scrollRef = useRef<HTMLDivElement>(null);
  const lanesRef = useRef<HTMLDivElement>(null);
  const vp = useViewport(scrollRef);
  const [regionDrag, setRegionDrag] = useState<RegionDrag | null>(null);
  const [trimDrag, setTrimDrag] = useState<TrimDrag | null>(null);
  const [slipDrag, setSlipDrag] = useState<SlipDrag | null>(null);
  const [marqueeDrag, setMarqueeDrag] = useState<MarqueeDrag | null>(null);
  const tool = useUi((s) => s.tool);
  const ripple = useUi((s) => s.ripple);
  const marquee = useUi((s) => s.marquee);
  const [trackDrag, setTrackDrag] = useState<TrackDrag | null>(null);
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; entries: MenuEntry[] } | null>(null);
  const [dropRow, setDropRow] = useState<number | null>(null);

  const map = useMemo(() => (project ? TempoMap.fromTransport(project.transport) : null), [project]);
  const rolling = isRolling(snapshot);
  const playhead = rolling && snapshot ? snapshot.ticks : cursorTicks;

  const snap = useCallback(
    (t: number) => {
      if (!project || !map) return Math.round(t);
      const ts = map.timeSignatureAt(Math.max(0, t));
      const spec = snapSpec ?? autoSpec(gridFor(ts, useUi.getState().pxPerQuarter).snapTicks, ticksPerBar(ts), ticksPerBeat(ts));
      return snapTick(map, t, spec);
    },
    [project, map, snapSpec],
  );

  // Ctrl/⌘ + wheel zooms around the pointer; native scrolling otherwise.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      const ui = useUi.getState();
      const rect = el.getBoundingClientRect();
      const anchor = Math.max(0, e.clientX - rect.left - ui.layout.trackHeaderWidth);
      const next = zoomAround(ui.pxPerQuarter, Math.exp(-e.deltaY * 0.0025), el.scrollLeft, anchor);
      ui.setZoom(next.pxPerQuarter);
      requestAnimationFrame(() => {
        el.scrollLeft = next.scrollLeft;
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // Follow the engine playhead: page the view when it leaves the screen.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !rolling || !follow) return;
    const x = ticksToPx(playhead, pxPerQuarter);
    const laneWidth = el.clientWidth - headerWidth;
    if (x < el.scrollLeft || x > el.scrollLeft + laneWidth - 24) el.scrollLeft = Math.max(0, x - 24);
  }, [playhead, rolling, follow, pxPerQuarter, headerWidth]);

  // Files dropped from the OS: import onto the audio track under the pointer
  // (sequentially), or onto new tracks.
  useEffect(() => {
    if (!inTauri) return;
    const target = (pos: { x: number; y: number }) => {
      const lanes = lanesRef.current;
      const p = useProject.getState().view?.project;
      if (!lanes || !p) return null;
      const ratio = window.devicePixelRatio || 1;
      const rect = lanes.getBoundingClientRect();
      const x = pos.x / ratio - rect.left;
      const y = pos.y / ratio - rect.top;
      if (x < 0 || y < 0 || x > rect.width) return null;
      const row = Math.floor(y / TRACK_HEIGHT);
      return { row, track: p.tracks[row], tick: pxToTicks(x, useUi.getState().pxPerQuarter) };
    };
    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      const ev = e.payload;
      if (ev.type === "over" || ev.type === "enter") {
        const t = target(ev.position);
        setDropRow(t ? t.row : null);
      } else if (ev.type === "leave") {
        setDropRow(null);
      } else if (ev.type === "drop") {
        setDropRow(null);
        const t = target(ev.position);
        const start = t ? snap(t.tick) : useUi.getState().cursorTicks;
        const onAudio = t?.track?.kind === "audio" ? t.track : null;
        void importAudioFiles(ev.paths, onAudio?.id ?? null, start, onAudio ? "sequential" : "newTracks");
      }
    });
    return () => void unlisten.then((f) => f());
  }, [snap]);

  const trackOrder = useMemo(() => project?.tracks.map((t) => t.id) ?? [], [project]);
  const regionIndex = useMemo(() => (project ? regionTrackIndex(project) : new Map<string, number>()), [project]);
  const mediaById = useMemo(() => new Map((project?.media ?? []).map((m) => [m.id, m])), [project]);

  const onSelectTrack = useCallback(
    (id: string, e: React.MouseEvent | React.PointerEvent) => {
      const ui = useUi.getState();
      const mode = e.shiftKey ? "range" : hasPrimaryModifier(e) ? "toggle" : "single";
      if (mode === "single" && ui.selection.trackIds.includes(id) && ui.selection.trackIds.length > 1 && e.type === "contextmenu") return;
      const next = nextTrackSelection(trackOrder, ui.selection, id, mode);
      ui.selectTracks(next.ids, next.anchor);
    },
    [trackOrder],
  );

  // No pointer capture here: it would retarget the double-click that starts
  // renaming. Moves and release are tracked on `window` instead.
  const onReorderStart = useCallback((e: ReactPointerEvent<HTMLDivElement>, id: string) => {
    if (e.shiftKey || hasPrimaryModifier(e)) return;
    setTrackDrag({ id, startY: e.clientY, moved: false, target: -1 });
  }, []);

  const selectRegion = useCallback(
    (region: Region, additive: boolean) => {
      const ui = useUi.getState();
      const trackIdx = regionIndex.get(region.id);
      const trackId = trackIdx !== undefined && project ? project.tracks[trackIdx]!.id : null;
      let ids = ui.selection.regionIds;
      if (additive) {
        ids = ids.includes(region.id) ? ids.filter((r) => r !== region.id) : [...ids, region.id];
        ui.selectRegions(ids);
      } else if (!ids.includes(region.id)) {
        ids = [region.id];
        ui.selectRegions(ids, trackId ? [trackId] : []);
      }
      return ids;
    },
    [project, regionIndex],
  );

  const onRegionPointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>, region: Region) => {
      if (e.button !== 0 || !project) return;
      e.stopPropagation();
      // Ctrl/Cmd+Alt drag slips the audio inside the region.
      if (hasPrimaryModifier(e) && e.altKey && region.content.type === "audio") {
        selectRegion(region, false);
        setSlipDrag({ region, startX: e.clientX, dTicks: 0 });
        return;
      }
      const ids = selectRegion(region, e.shiftKey || hasPrimaryModifier(e));
      if (!ids.includes(region.id)) return;
      setRegionDrag({ ids, primary: region, startX: e.clientX, startY: e.clientY, moved: false, dTicks: 0, dTracks: 0, valid: true });
    },
    [project, selectRegion],
  );

  const onTrimStart = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>, region: Region, edge: "start" | "end") => {
      if (e.button !== 0) return;
      e.stopPropagation();
      selectRegion(region, false);
      setTrimDrag({ region, edge, startX: e.clientX, start: region.start, end: region.start + region.length, moved: false });
    },
    [selectRegion],
  );

  const onRegionContextMenu = useCallback(
    (e: React.MouseEvent<HTMLDivElement>, region: Region) => {
      e.preventDefault();
      e.stopPropagation();
      const ids = useUi.getState().selection.regionIds.includes(region.id) ? useUi.getState().selection.regionIds : selectRegion(region, false);
      const audio = region.content.type === "audio" ? region.content : null;
      const entries: MenuEntry[] = [
        { heading: ids.length > 1 ? `${ids.length} regions` : region.name },
        { id: "split", label: "Split at playhead", icon: "split", onSelect: () => void splitSelectedAtPlayhead() },
        { id: "dup", label: "Duplicate", icon: "duplicate", onSelect: () => void duplicateSelectedRegions() },
        { id: "glue", label: "Glue", shortcut: "Ctrl+J", disabled: ids.length < 2, onSelect: () => void glueSelected() },
        "separator",
        { id: "cut", label: "Cut", onSelect: () => void cutSelectedRegions() },
        { id: "copy", label: "Copy", onSelect: () => void copySelectedRegions() },
        { id: "paste", label: "Paste at playhead", icon: "paste", disabled: clipboardSize() === 0, onSelect: () => void pasteRegions() },
        "separator",
        { id: "mute", label: region.muted ? "Unmute" : "Mute", icon: "muteRegion", onSelect: () => void setSelectedRegionsMuted(!region.muted) },
      ];
      if (audio?.media) {
        entries.push({
          id: "loop",
          label: audio.looped ? "Stop looping" : "Loop the file",
          icon: "loop",
          onSelect: () =>
            void apply({ type: "setAudioRegion", id: region.id, gainDb: audio.gainDb, fadeIn: audio.fadeIn, fadeOut: audio.fadeOut, looped: !audio.looped }),
        });
      }
      if (audio) {
        entries.push(
          "separator",
          { id: "reverse", label: audio.reversed ? "Play forwards" : "Reverse", onSelect: () => void reverseSelected() },
          { id: "normalize", label: "Normalize to −1 dBFS", onSelect: () => void normalizeSelected(-1) },
          { id: "xfade", label: "Crossfade with next region (20 ms)", onSelect: () => void crossfadeWithNext(20) },
          { id: "fadeEp", label: `Fade shape: equal power${audio.fadeCurve === "equalPower" ? " ✓" : ""}`, onSelect: () => void setFadeCurve("equalPower") },
          { id: "fadeLin", label: `Fade shape: linear${audio.fadeCurve === "linear" ? " ✓" : ""}`, onSelect: () => void setFadeCurve("linear") },
          { id: "fadeS", label: `Fade shape: S-curve${audio.fadeCurve === "sCurve" ? " ✓" : ""}`, onSelect: () => void setFadeCurve("sCurve") },
          { id: "slice", label: "Slice at transients", onSelect: () => void sliceAtTransients(0.5) },
          { id: "markers", label: "Add markers at transients", onSelect: () => void markersAtTransients(0.5) },
          { id: "bounce", label: "Bounce in place", onSelect: () => void bounceSelected(false) },
        );
      }
      entries.push("separator", { id: "ripple", label: "Ripple delete", onSelect: () => void rippleDeleteSelected() }, {
        id: "delete",
        label: "Delete",
        icon: "trash",
        danger: true,
        onSelect: () => void apply({ type: "removeRegions", ids }),
      });
      setMenu({ at: { x: e.clientX, y: e.clientY }, entries });
    },
    [apply, selectRegion],
  );

  // Pointer moves/ups for drags are tracked on window so they keep working
  // when the pointer leaves the element that captured it.
  useEffect(() => {
    if (!regionDrag && !trackDrag && !trimDrag && !slipDrag && !marqueeDrag) return;
    const onMove = (e: PointerEvent) => {
      if (!project) return;
      const zoom = useUi.getState().pxPerQuarter;
      if (slipDrag) {
        setSlipDrag({ ...slipDrag, dTicks: Math.round(pxToTicks(e.clientX - slipDrag.startX, zoom)) });
        return;
      }
      if (marqueeDrag) {
        const lanes = lanesRef.current;
        if (!lanes) return;
        const rect = lanes.getBoundingClientRect();
        const tick = Math.max(0, pxToTicks(e.clientX - rect.left, zoom));
        const row = Math.max(0, Math.min(project.tracks.length - 1, Math.floor((e.clientY - rect.top) / TRACK_HEIGHT)));
        const [r0, r1] = [Math.min(row, marqueeDrag.startRow), Math.max(row, marqueeDrag.startRow)];
        const a = e.altKey ? Math.round(marqueeDrag.startTick) : snap(marqueeDrag.startTick);
        const b = e.altKey ? Math.round(tick) : snap(tick);
        useUi.getState().setMarquee({
          trackIds: project.tracks.slice(r0, r1 + 1).map((t) => t.id),
          start: Math.min(a, b),
          end: Math.max(a, b),
        });
        return;
      }
      if (trimDrag) {
        const dx = e.clientX - trimDrag.startX;
        if (!trimDrag.moved && Math.abs(dx) < 3) return;
        const raw = (trimDrag.edge === "start" ? trimDrag.region.start : trimDrag.region.start + trimDrag.region.length) + pxToTicks(dx, zoom);
        const t = Math.max(0, e.altKey ? Math.round(raw) : snap(raw));
        const end0 = trimDrag.region.start + trimDrag.region.length;
        setTrimDrag({
          ...trimDrag,
          moved: true,
          start: trimDrag.edge === "start" ? Math.min(t, end0 - 1) : trimDrag.region.start,
          end: trimDrag.edge === "end" ? Math.max(t, trimDrag.region.start + 1) : end0,
        });
      } else if (regionDrag) {
        const dx = e.clientX - regionDrag.startX;
        const dy = e.clientY - regionDrag.startY;
        if (!regionDrag.moved && Math.hypot(dx, dy) < 4) return;
        const raw = regionDrag.primary.start + pxToTicks(dx, zoom);
        const target = e.altKey ? Math.round(raw) : snap(raw);
        const dTicks = Math.max(-regionDrag.primary.start, target - regionDrag.primary.start);
        const dTracks = Math.round(dy / TRACK_HEIGHT);
        setRegionDrag({ ...regionDrag, moved: true, dTicks, dTracks, valid: canMove(project, regionDrag.ids, regionIndex, dTracks) });
      } else if (trackDrag) {
        const dy = e.clientY - trackDrag.startY;
        if (!trackDrag.moved && Math.abs(dy) < 5) return;
        const from = project.tracks.findIndex((t) => t.id === trackDrag.id);
        const target = Math.max(0, Math.min(project.tracks.length, from + Math.round(dy / TRACK_HEIGHT) + (dy > 0 ? 1 : 0)));
        setTrackDrag({ ...trackDrag, moved: true, target });
      }
    };
    const onUp = () => {
      if (slipDrag && slipDrag.dTicks !== 0) {
        void apply({ type: "slipRegions", ids: [slipDrag.region.id], deltaTicks: slipDrag.dTicks });
      }
      if (marqueeDrag) {
        const m = useUi.getState().marquee;
        if (m && m.end - m.start < 1) useUi.getState().setMarquee(null);
      }
      setSlipDrag(null);
      setMarqueeDrag(null);
      if (trimDrag?.moved) {
        const r = trimDrag.region;
        if (trimDrag.start !== r.start || trimDrag.end !== r.start + r.length) {
          void apply({ type: "trimRegion", id: r.id, start: trimDrag.start, end: trimDrag.end });
        }
      }
      if (regionDrag?.moved && project) {
        if (!regionDrag.valid) {
          notify("warning", "Regions can only move onto tracks of a matching type (audio ↔ audio, MIDI ↔ instrument/MIDI).");
        } else if (regionDrag.dTicks !== 0 || regionDrag.dTracks !== 0) {
          // Earliest region in the group clamps at zero in the core as well.
          void apply({ type: "moveRegions", ids: regionDrag.ids, deltaTicks: regionDrag.dTicks, trackOffset: regionDrag.dTracks });
        }
      }
      if (trackDrag?.moved && project && trackDrag.target >= 0) {
        const from = project.tracks.findIndex((t) => t.id === trackDrag.id);
        const to = trackDrag.target > from ? trackDrag.target - 1 : trackDrag.target;
        if (to !== from) void apply({ type: "moveTrack", id: trackDrag.id, toIndex: to });
      }
      setRegionDrag(null);
      setTrackDrag(null);
      setTrimDrag(null);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  }, [regionDrag, trackDrag, trimDrag, slipDrag, marqueeDrag, project, regionIndex, apply, snap]);

  if (!project || !map) return <div className="timeline" />;

  const ts = project.transport.timeSignature;
  const grid = gridFor(ts, pxPerQuarter);
  const lastBar = map.barStart(project.lengthBars);
  const contentTicks = Math.max(lastBar, pxToTicks(vp.width, pxPerQuarter));
  const contentWidth = Math.ceil(ticksToPx(contentTicks, pxPerQuarter)) + 200;
  const lanesHeight = Math.max(project.tracks.length * TRACK_HEIGHT, vp.height - RULER_HEIGHT);
  const laneLeft = vp.left - headerWidth;
  const range = visibleTicks(Math.max(0, laneLeft), vp.width, pxPerQuarter);
  const anySolo = project.tracks.some((t) => t.soloed);
  const selectedTracks = new Set(selection.trackIds);
  const selectedRegions = new Set(selection.regionIds);
  const dragIds = regionDrag?.moved ? new Set(regionDrag.ids) : null;
  const dragOffset = regionDrag?.moved ? { x: ticksToPx(regionDrag.dTicks, pxPerQuarter), y: regionDrag.dTracks * TRACK_HEIGHT } : null;
  const loop = project.transport.loopRange;
  const effectiveSpec = snapSpec ?? autoSpec(grid.snapTicks, ticksPerBar(ts), ticksPerBeat(ts));

  const audioDisplay = (r: Region, left: number, width: number): AudioDisplay | undefined => {
    if (r.content.type !== "audio" || !r.content.media) return undefined;
    const c = r.content;
    const asset = mediaById.get(c.media!);
    const state = mediaStates[c.media!];
    const status: AudioDisplay["status"] =
      state?.state === "ready" ? "ready" : state?.state === "failed" ? "failed" : asset ? "loading" : "failed";
    const rate = asset?.info?.sampleRate ?? project.sampleRate;
    const s0 = map.secondsAt(r.start);
    const fadeInTick = map.tickAtSeconds(s0 + c.fadeIn / rate);
    const fadeOutTick = map.tickAtSeconds(Math.max(s0, map.secondsAt(r.start + r.length) - c.fadeOut / rate));
    const visFrom = Math.max(r.start, range.start);
    const visTo = Math.min(r.start + r.length, range.end);
    return {
      status,
      message:
        state?.state === "failed"
          ? state.code === "missing"
            ? `Missing file: ${asset?.path ?? ""}`
            : state.message
          : !asset
            ? "File not in the media pool"
            : undefined,
      fadeInPx: Math.min(width, ticksToPx(fadeInTick, pxPerQuarter) - left),
      fadeOutPx: Math.max(0, ticksToPx(fadeOutTick, pxPerQuarter) - left),
      visibleLeftPx: ticksToPx(visFrom, pxPerQuarter) - left,
      visibleWidthPx: visTo > visFrom ? ticksToPx(visTo - visFrom, pxPerQuarter) : 0,
      fromTick: visFrom,
      toTick: visTo,
      revision: `${c.media}|${r.start}|${r.length}|${c.offset}|${c.gainDb}|${c.looped}|${project.transport.tempo}|${project.transport.tempoChanges.length}|${state?.state === "ready" ? (state.changed ? "c" : "r") : "x"}`,
      looped: c.looped,
    };
  };

  const onLanePointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const row = Math.floor((e.clientY - rect.top) / TRACK_HEIGHT);
    if (tool === "marquee" && project.tracks[row]) {
      const tick = Math.max(0, pxToTicks(e.clientX - rect.left, pxPerQuarter));
      useUi.getState().setMarquee(null);
      setMarqueeDrag({ startTick: tick, startRow: row });
      return;
    }
    useUi.getState().setMarquee(null);
    const track = project.tracks[row];
    const ui = useUi.getState();
    if (track) {
      const next = nextTrackSelection(trackOrder, ui.selection, track.id, e.shiftKey ? "range" : hasPrimaryModifier(e) ? "toggle" : "single");
      ui.selectTracks(next.ids, next.anchor);
    } else {
      ui.clearSelection();
    }
  };

  const onLaneDoubleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const track = project.tracks[Math.floor((e.clientY - rect.top) / TRACK_HEIGHT)];
    if (!track) return;
    const t = pxToTicks(e.clientX - rect.left, pxPerQuarter);
    if (track.kind === "audio") {
      useUi.getState().setCursor(snap(t));
      useUi.getState().selectTracks([track.id], track.id);
      void importAudioDialog();
      return;
    }
    const bar = map.barAt(t);
    void apply({ type: "createMidiRegion", trackId: track.id, start: bar.start, length: ticksPerBar(bar.ts) }).then((o) => {
      const id = o?.createdRegions[0];
      if (id) useUi.getState().selectRegions([id], [track.id]);
    });
  };

  const dropLineTop = trackDrag?.moved && trackDrag.target >= 0 ? trackDrag.target * TRACK_HEIGHT : null;

  return (
    <section className="timeline" aria-label="Arrangement">
      <div className="timeline__toolbar">
        <ArrangementMenus />
        <div className="segmented timeline__tools" role="group" aria-label="Arrangement tool">
          <button
            type="button"
            className={tool === "pointer" ? "is-on" : ""}
            aria-pressed={tool === "pointer"}
            title="Pointer: select, move and trim regions. Ctrl+Alt-drag slips the audio inside a region."
            onClick={() => useUi.getState().setTool("pointer")}
          >
            Pointer
          </button>
          <button
            type="button"
            className={tool === "marquee" ? "is-on" : ""}
            aria-pressed={tool === "marquee"}
            title="Marquee: drag a time range over tracks, then right-click to split, delete or ripple-delete it"
            onClick={() => useUi.getState().setTool("marquee")}
          >
            Marquee
          </button>
        </div>
        <button
          type="button"
          className={`pr__btn timeline__ripple${ripple ? " is-on" : ""}`}
          aria-pressed={ripple}
          title="Ripple: deleting regions or ranges closes the gap on their tracks"
          onClick={() => useUi.getState().setRipple(!ripple)}
        >
          Ripple {ripple ? "on" : "off"}
        </button>
        <div className="timeline__toolbar-right">
          <label className="hint" title="Snap grid for moving, trimming and the cursor. Hold Alt while dragging for no snapping.">
            Snap{" "}
            <select
              aria-label="Snap grid"
              value={SNAP_CHOICES.findIndex((c) => JSON.stringify(c) === JSON.stringify(snapSpec))}
              onChange={(e) => setSnapSpec(SNAP_CHOICES[Number(e.target.value)] ?? null)}
            >
              {SNAP_CHOICES.map((c, i) => (
                <option key={i} value={i}>
                  {c ? specLabel(c) : `Auto (${specLabel(effectiveSpec)})`}
                </option>
              ))}
            </select>
          </label>
          <IconButton icon="zoomOut" label="Zoom out" onClick={() => setZoom(pxPerQuarter / 1.25)} size={14} />
          <input
            className="zoom-slider"
            type="range"
            aria-label="Horizontal zoom"
            min={Math.log(MIN_PX_PER_QUARTER)}
            max={Math.log(MAX_PX_PER_QUARTER)}
            step={0.01}
            value={Math.log(pxPerQuarter)}
            onChange={(e) => setZoom(Math.exp(Number(e.target.value)))}
          />
          <IconButton icon="zoomIn" label="Zoom in" onClick={() => setZoom(pxPerQuarter * 1.25)} size={14} />
        </div>
      </div>

      <div className="timeline__scroll" ref={scrollRef}>
        <div className="timeline__content" style={{ width: headerWidth + contentWidth }}>
          <div className="timeline__top" style={{ height: RULER_HEIGHT }}>
            <div className="timeline__corner" style={{ width: headerWidth }}>
              <AddTrackButton />
              <IconButton
                icon="duplicate"
                label="Duplicate selected tracks"
                size={13}
                disabled={selectedTracks.size === 0}
                onClick={() => void duplicateSelectedTracks()}
              />
              <span className="timeline__track-count">{project.tracks.length} tracks</span>
            </div>
            <Ruler
              transport={project.transport}
              markers={project.markers}
              map={map}
              width={contentWidth}
              pxPerQuarter={pxPerQuarter}
              viewLeft={Math.max(0, laneLeft)}
              viewWidth={vp.width}
              cursorTicks={playhead}
              snap={snap}
            />
          </div>

          <div className="timeline__body" style={{ height: lanesHeight }}>
            <div className="track-headers" style={{ width: headerWidth, height: lanesHeight }} role="grid" aria-label="Tracks">
              {project.tracks.map((t, i) => (
                <TrackHeader
                  key={t.id}
                  track={t}
                  index={i}
                  height={TRACK_HEIGHT}
                  selected={selectedTracks.has(t.id)}
                  audible={!t.muted && (!anySolo || t.soloed)}
                  onSelect={onSelectTrack}
                  onDuplicate={() => void duplicateSelectedTracks()}
                  onDelete={() => void deleteSelectedTracks()}
                  onReorderStart={onReorderStart}
                />
              ))}
              {dropLineTop !== null && <div className="drop-line" style={{ top: dropLineTop }} />}
            </div>

            <div
              ref={lanesRef}
              className="lanes"
              style={{
                width: contentWidth,
                height: lanesHeight,
                ...gridBackground([{ axis: "y", every: TRACK_HEIGHT, color: "var(--grid-row)" }]),
              }}
              onPointerDown={onLanePointerDown}
              onDoubleClick={onLaneDoubleClick}
              data-testid="lanes"
            >
              <MeterGrid
                map={map}
                pxPerQuarter={pxPerQuarter}
                width={contentWidth}
                className="lanes__grid"
                layers={(g) => [
                  { axis: "x", every: g.barPx, color: "var(--grid-bar)" },
                  { axis: "x", every: g.beatPx, color: "var(--grid-beat)" },
                  { axis: "x", every: g.subPx, color: "var(--grid-sub)" },
                ]}
              />
              {project.tracks.map((t, i) =>
                selectedTracks.has(t.id) || dropRow === i ? (
                  <div
                    key={t.id}
                    className={`lane-highlight${dropRow === i ? " is-drop" : ""}`}
                    style={{ top: i * TRACK_HEIGHT, height: TRACK_HEIGHT }}
                  />
                ) : null,
              )}
              {loop.enabled && (
                <div
                  className="loop-shade"
                  style={{ left: ticksToPx(loop.start, pxPerQuarter), width: ticksToPx(loop.end - loop.start, pxPerQuarter) }}
                />
              )}
              {project.tracks.map((t, i) => {
                const silent = t.muted || (anySolo && !t.soloed);
                return t.regions.map((r) => {
                  const trimming = trimDrag?.moved && trimDrag.region.id === r.id ? trimDrag : null;
                  const start = trimming ? trimming.start : r.start;
                  const end = trimming ? trimming.end : r.start + r.length;
                  const dragging = dragIds?.has(r.id) ?? false;
                  if (!dragging && (end < range.start || start > range.end)) return null;
                  const left = ticksToPx(start, pxPerQuarter);
                  const width = ticksToPx(end - start, pxPerQuarter);
                  return (
                    <RegionView
                      key={r.id}
                      region={r}
                      color={t.color}
                      left={left}
                      width={width}
                      top={i * TRACK_HEIGHT + 2}
                      height={TRACK_HEIGHT - 4}
                      selected={selectedRegions.has(r.id)}
                      dimmed={silent || r.muted}
                      demo={demo}
                      offset={dragging ? dragOffset : null}
                      invalid={dragging && regionDrag !== null && !regionDrag.valid}
                      audio={trimming ? undefined : audioDisplay(r, left, width)}
                      onPointerDown={onRegionPointerDown}
                      onTrimStart={onTrimStart}
                      onContextMenu={onRegionContextMenu}
                    />
                  );
                });
              })}
              {project.markers.map((m) => (
                <div key={m.id} className="marker-line" style={{ left: ticksToPx(m.tick, pxPerQuarter) }} />
              ))}
              {marquee && (
                <MarqueeView
                  marquee={marquee}
                  rows={marquee.trackIds.map((id) => project.tracks.findIndex((t) => t.id === id)).filter((i) => i >= 0)}
                  pxPerQuarter={pxPerQuarter}
                  onMenu={(at) =>
                    setMenu({
                      at,
                      entries: [
                        { heading: `Range ${map.formatPosition(marquee.start)} – ${map.formatPosition(marquee.end)}` },
                        { id: "split", label: "Split at range (select inside)", onSelect: () => void splitMarquee() },
                        {
                          id: "del",
                          label: "Delete range",
                          onSelect: () =>
                            void apply({ type: "cutRange", trackIds: marquee.trackIds, start: marquee.start, end: marquee.end, ripple: false }).then(() =>
                              useUi.getState().setMarquee(null),
                            ),
                        },
                        {
                          id: "ripple",
                          label: "Ripple delete range",
                          onSelect: () =>
                            void apply({ type: "cutRange", trackIds: marquee.trackIds, start: marquee.start, end: marquee.end, ripple: true }).then(() =>
                              useUi.getState().setMarquee(null),
                            ),
                        },
                      ],
                    })
                  }
                />
              )}
              {slipDrag && (
                <div
                  className="slip-badge"
                  style={{ left: ticksToPx(slipDrag.region.start, pxPerQuarter), top: (regionIndex.get(slipDrag.region.id) ?? 0) * TRACK_HEIGHT }}
                >
                  {`Slip ${slipDrag.dTicks >= 0 ? "+" : "−"}${Math.abs(map.secondsAt(slipDrag.region.start + slipDrag.dTicks) - map.secondsAt(slipDrag.region.start)).toFixed(3)} s`}
                </div>
              )}
              <div
                className={`cursor-line${rolling ? " is-playing" : ""}`}
                style={{ transform: `translateX(${ticksToPx(playhead, pxPerQuarter)}px)` }}
                data-testid="playhead"
              />
            </div>
          </div>
        </div>
      </div>
      {project.tracks.length === 0 && (
        <div className="timeline-empty" style={{ left: headerWidth, top: 34 + RULER_HEIGHT }}>
          <EmptyProject />
        </div>
      )}
      {menu && <Menu at={menu.at} entries={menu.entries} onClose={() => setMenu(null)} />}
    </section>
  );
}
