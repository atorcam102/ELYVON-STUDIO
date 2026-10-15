import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import { Button, IconButton } from "../../components/Button";
import {
  gridFor,
  MAX_PX_PER_QUARTER,
  MIN_PX_PER_QUARTER,
  pxToTicks,
  RULER_HEIGHT,
  snap,
  ticksToPx,
  TRACK_HEIGHT,
  visibleTicks,
  zoomAround,
} from "../../lib/geometry";
import { gridBackground } from "../../lib/gridBackground";
import { PPQ, ticksPerBar, ticksPerBeat } from "../../lib/music";
import { hasPrimaryModifier } from "../../lib/platform";
import { notify } from "../../state/notifications";
import { useProject } from "../../state/project";
import { nextTrackSelection, useUi } from "../../state/ui";
import { addTrack, duplicateSelectedTracks, deleteSelectedTracks } from "../project/editActions";
import { openDemoProject } from "../project/projectActions";
import { AddTrackButton } from "../tracks/AddTrackMenu";
import { TrackHeader } from "../tracks/TrackHeader";
import { RegionView } from "./RegionView";
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
    const isAudio = region.content.type === "audio";
    if (isAudio !== (target.kind === "audio")) return false;
  }
  return true;
}

function EmptyProject() {
  return (
    <>
      <div className="timeline-empty__card">
        <h3>Start with a track</h3>
        <p>Choose the kind of track to add. You can change names, colours and order at any time.</p>
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
        </div>
        <button type="button" className="link-btn" onClick={() => void openDemoProject()}>
          Or explore the demo session
        </button>
      </div>
    </>
  );
}

export function Timeline() {
  const project = useProject((s) => s.view?.project);
  const apply = useProject((s) => s.apply);
  const pxPerQuarter = useUi((s) => s.pxPerQuarter);
  const setZoom = useUi((s) => s.setZoom);
  const cursorTicks = useUi((s) => s.cursorTicks);
  const selection = useUi((s) => s.selection);
  const headerWidth = useUi((s) => s.layout.trackHeaderWidth);
  const scrollRef = useRef<HTMLDivElement>(null);
  const vp = useViewport(scrollRef);
  const [regionDrag, setRegionDrag] = useState<RegionDrag | null>(null);
  const [trackDrag, setTrackDrag] = useState<TrackDrag | null>(null);

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

  const trackOrder = useMemo(() => project?.tracks.map((t) => t.id) ?? [], [project]);
  const regionIndex = useMemo(() => (project ? regionTrackIndex(project) : new Map<string, number>()), [project]);

  const onSelectTrack = useCallback(
    (id: string, e: React.MouseEvent | React.PointerEvent) => {
      const ui = useUi.getState();
      const mode = e.shiftKey ? "range" : hasPrimaryModifier(e) ? "toggle" : "single";
      if (mode === "single" && ui.selection.trackIds.includes(id) && ui.selection.trackIds.length > 1 && e.type === "contextmenu") {
        return;
      }
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

  const onRegionPointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>, region: Region) => {
      if (e.button !== 0 || !project) return;
      e.stopPropagation();
      const ui = useUi.getState();
      const trackIdx = regionIndex.get(region.id);
      const trackId = trackIdx !== undefined ? project.tracks[trackIdx]!.id : null;
      let ids = ui.selection.regionIds;
      if (e.shiftKey || hasPrimaryModifier(e)) {
        ids = ids.includes(region.id) ? ids.filter((r) => r !== region.id) : [...ids, region.id];
        ui.selectRegions(ids);
        if (!ids.includes(region.id)) return;
      } else if (!ids.includes(region.id)) {
        ids = [region.id];
        ui.selectRegions(ids, trackId ? [trackId] : []);
      }
      setRegionDrag({ ids, primary: region, startX: e.clientX, startY: e.clientY, moved: false, dTicks: 0, dTracks: 0, valid: true });
    },
    [project, regionIndex],
  );

  // Pointer moves/ups for drags are tracked on window so they keep working
  // when the pointer leaves the element that captured it.
  useEffect(() => {
    if (!regionDrag && !trackDrag) return;
    const onMove = (e: PointerEvent) => {
      if (!project) return;
      if (regionDrag) {
        const dx = e.clientX - regionDrag.startX;
        const dy = e.clientY - regionDrag.startY;
        if (!regionDrag.moved && Math.hypot(dx, dy) < 4) return;
        const g = gridFor(project.transport.timeSignature, useUi.getState().pxPerQuarter);
        const raw = pxToTicks(dx, useUi.getState().pxPerQuarter);
        const fine = e.altKey ? PPQ / 16 : g.snapTicks;
        const dTicks = Math.max(-regionDrag.primary.start, snap(regionDrag.primary.start + raw, fine) - regionDrag.primary.start);
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
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  }, [regionDrag, trackDrag, project, regionIndex, apply]);

  if (!project) return <div className="timeline" />;

  const ts = project.transport.timeSignature;
  const grid = gridFor(ts, pxPerQuarter);
  const barTicks = ticksPerBar(ts);
  const contentTicks = Math.max(project.lengthBars * barTicks, pxToTicks(vp.width, pxPerQuarter));
  const contentWidth = Math.ceil(ticksToPx(contentTicks, pxPerQuarter)) + 200;
  const lanesHeight = Math.max(project.tracks.length * TRACK_HEIGHT, vp.height - RULER_HEIGHT);
  const laneLeft = vp.left - headerWidth;
  const range = visibleTicks(Math.max(0, laneLeft), vp.width, pxPerQuarter);
  const anySolo = project.tracks.some((t) => t.soloed);
  const selectedTracks = new Set(selection.trackIds);
  const selectedRegions = new Set(selection.regionIds);
  const dragIds = regionDrag?.moved ? new Set(regionDrag.ids) : null;
  const dragOffset = regionDrag?.moved
    ? { x: ticksToPx(regionDrag.dTicks, pxPerQuarter), y: regionDrag.dTracks * TRACK_HEIGHT }
    : null;
  const loop = project.transport.loopRange;

  const gridStyle = gridBackground([
    { axis: "x", every: grid.barPx, color: "var(--grid-bar)" },
    { axis: "x", every: grid.beatPx, color: "var(--grid-beat)" },
    { axis: "x", every: grid.subPx, color: "var(--grid-sub)" },
    { axis: "y", every: TRACK_HEIGHT, color: "var(--grid-row)" },
  ]);

  const onLanePointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const row = Math.floor((e.clientY - rect.top) / TRACK_HEIGHT);
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
    if (track.kind === "audio") {
      notify("info", "Audio regions come from recording or importing files, which arrive with the audio engine (Phase 2).");
      return;
    }
    const t = pxToTicks(e.clientX - rect.left, pxPerQuarter);
    const start = Math.floor(t / barTicks) * barTicks;
    void apply({ type: "createMidiRegion", trackId: track.id, start, length: barTicks }).then((o) => {
      const id = o?.createdRegions[0];
      if (id) useUi.getState().selectRegions([id], [track.id]);
    });
  };

  const dropLineTop = trackDrag?.moved && trackDrag.target >= 0 ? trackDrag.target * TRACK_HEIGHT : null;

  return (
    <section className="timeline" aria-label="Arrangement">
      <div className="timeline__toolbar">
        <div className="timeline__toolbar-left" style={{ width: headerWidth }}>
          <span className="panel-title">Tracks</span>
          <span className="count">{project.tracks.length}</span>
          <AddTrackButton />
        </div>
        <div className="timeline__toolbar-right">
          <span className="hint" title="Grid resolution used for snapping. Hold Alt while dragging for fine positioning.">
            Snap {grid.snapTicks >= barTicks ? "1 bar" : grid.snapTicks >= ticksPerBeat(ts) ? `1/${ts.denominator}` : `1/${Math.round((PPQ * 4) / grid.snapTicks)}`}
          </span>
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
            <div className="timeline__corner" style={{ width: headerWidth }} />
            <Ruler
              transport={project.transport}
              width={contentWidth}
              lengthBars={Math.ceil(contentTicks / barTicks)}
              pxPerQuarter={pxPerQuarter}
              grid={grid}
              viewLeft={Math.max(0, laneLeft)}
              viewWidth={vp.width}
              cursorTicks={cursorTicks}
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
              className="lanes"
              style={{ width: contentWidth, height: lanesHeight, ...gridStyle }}
              onPointerDown={onLanePointerDown}
              onDoubleClick={onLaneDoubleClick}
            >
              {project.tracks.map((t, i) =>
                selectedTracks.has(t.id) ? (
                  <div key={t.id} className="lane-highlight" style={{ top: i * TRACK_HEIGHT, height: TRACK_HEIGHT }} />
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
                  const end = r.start + r.length;
                  const dragging = dragIds?.has(r.id) ?? false;
                  if (!dragging && (end < range.start || r.start > range.end)) return null;
                  return (
                    <RegionView
                      key={r.id}
                      region={r}
                      color={t.color}
                      left={ticksToPx(r.start, pxPerQuarter)}
                      width={ticksToPx(r.length, pxPerQuarter)}
                      top={i * TRACK_HEIGHT + 2}
                      height={TRACK_HEIGHT - 4}
                      selected={selectedRegions.has(r.id)}
                      dimmed={silent}
                      offset={dragging ? dragOffset : null}
                      invalid={dragging && regionDrag !== null && !regionDrag.valid}
                      onPointerDown={onRegionPointerDown}
                    />
                  );
                });
              })}
              <div className="cursor-line" style={{ left: ticksToPx(cursorTicks, pxPerQuarter) }} />
            </div>
          </div>
        </div>
      </div>
      {project.tracks.length === 0 && (
        <div className="timeline-empty" style={{ left: headerWidth, top: 34 + RULER_HEIGHT }}>
          <EmptyProject />
        </div>
      )}
    </section>
  );
}
