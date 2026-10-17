// Audio editor tab (Phase 11): take folders and comping, plus the
// non-destructive tools of a plain audio region. Drag across a take to make
// that part of the folder play from it; the takes are never changed.

import { useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import type { TakeComp } from "../../bindings/TakeComp";
import type { Track } from "../../bindings/Track";
import { Button } from "../../components/Button";
import { NumberField } from "../../components/Fields";
import { Icon } from "../../components/Icon";
import { snapTick, TempoMap } from "../../lib/tempoMap";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { RegionWaveform } from "../timeline/RegionWaveform";
import {
  bounceSelected,
  markersAtTransients,
  normalizeSelected,
  reverseSelected,
  sliceAtTransients,
} from "../project/editingActions";

const TAKE_COLORS = ["#365DCD", "#3FA75A", "#C99A22", "#C9455A", "#8E4FD1", "#19A7C0", "#D96F2B", "#22BCBC"];
const ROW_H = 46;
const LABEL_W = 92;

function selectedAudio(project: Project, ids: string[]): { region: Region; track: Track } | null {
  for (let i = ids.length - 1; i >= 0; i--) {
    for (const t of project.tracks) {
      const r = t.regions.find((x) => x.id === ids[i]);
      if (r?.content.type === "audio") return { region: r, track: t };
    }
  }
  return null;
}

export function AudioEditor() {
  const project = useProject((s) => s.view?.project);
  const regionIds = useUi((s) => s.selection.regionIds);
  const found = project ? selectedAudio(project, regionIds) : null;
  if (!project || !found) {
    return (
      <div className="editor-empty">
        <Icon name="waveform" size={22} />
        <p>Select an audio region or a recorded take.</p>
        <span>Cycle recordings create take folders: comp them here by dragging across the takes.</span>
      </div>
    );
  }
  if (found.region.capture) return <TakeFolder project={project} track={found.track} folder={found.region.capture.folder} />;
  return <RegionTools project={project} region={found.region} />;
}

function RegionTools({ project, region }: { project: Project; region: Region }) {
  if (region.content.type !== "audio") return null;
  const c = region.content;
  const asset = project.media.find((m) => m.id === c.media);
  return (
    <div className="takes">
      <div className="pr__bar">
        <span className="panel-title">{region.name}</span>
        <span className="hint">{asset ? asset.path : "No file (placeholder)"}</span>
        <div className="pr__bar-end">
          <button type="button" className="pr__btn" onClick={() => void reverseSelected()}>
            {c.reversed ? "Play forwards" : "Reverse"}
          </button>
          <button type="button" className="pr__btn" onClick={() => void normalizeSelected(-1)}>
            Normalize −1 dBFS
          </button>
          <button type="button" className="pr__btn" onClick={() => void sliceAtTransients(0.5)}>
            Slice at transients
          </button>
          <button type="button" className="pr__btn" onClick={() => void markersAtTransients(0.5)}>
            Markers at transients
          </button>
          <button type="button" className="pr__btn" onClick={() => void bounceSelected(false)}>
            Bounce in place
          </button>
        </div>
      </div>
      <p className="takes__note">
        Every edit here is non-destructive: gain, direction, fades and slices only change how the region plays its file. Ctrl+Alt-drag a region in
        the arrangement to slip its audio.
      </p>
    </div>
  );
}

interface Pick {
  take: number;
  from: number;
  to: number;
}

function TakeFolder({ project, track, folder }: { project: Project; track: Track; folder: string }) {
  const apply = useProject((s) => s.apply);
  const map = TempoMap.fromTransport(project.transport);
  const takes = track.regions
    .filter((r) => r.capture?.folder === folder)
    .sort((a, b) => (a.capture?.take ?? 0) - (b.capture?.take ?? 0));
  const comp: TakeComp | undefined = track.comps.find((c) => c.folder === folder);
  const start = Math.min(...takes.map((r) => r.start));
  const end = Math.max(...takes.map((r) => r.start + r.length));
  const bodyRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(600);
  const [pick, setPick] = useState<Pick | null>(null);
  const drag = useRef<{ take: number; x0: number } | null>(null);
  useLayoutEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const fit = () => setWidth(Math.max(100, el.clientWidth - LABEL_W - 8));
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const span = Math.max(1, end - start);
  const x = (t: number) => ((t - start) / span) * width;
  const tickAt = (px: number, free: boolean) => {
    const t = start + (Math.min(width, Math.max(0, px)) / width) * span;
    return free ? Math.round(t) : Math.min(end, Math.max(start, snapTick(map, t, { division: "sixteenth", triplet: false, swing: 0 })));
  };
  const active = takes.find((r) => !r.muted)?.capture?.take;
  const segments = comp && !comp.flattened ? comp.segments : active ? [{ start, end, take: active }] : [];
  const color = (take: number) => TAKE_COLORS[(take - 1) % TAKE_COLORS.length]!;

  const local = (e: { clientX: number }, el: HTMLElement) => e.clientX - el.getBoundingClientRect().left;
  const down = (e: ReactPointerEvent<HTMLDivElement>, take: number) => {
    if (e.button !== 0 || comp?.flattened) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const px = local(e, e.currentTarget);
    drag.current = { take, x0: px };
    const t = tickAt(px, e.shiftKey);
    setPick({ take, from: t, to: t });
  };
  const move = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const a = tickAt(d.x0, e.shiftKey);
    const b = tickAt(local(e, e.currentTarget), e.shiftKey);
    setPick({ take: d.take, from: Math.min(a, b), to: Math.max(a, b) });
  };
  const up = () => {
    const p = pick;
    drag.current = null;
    setPick(null);
    if (p && p.to > p.from) void apply({ type: "compTake", trackId: track.id, folder, take: p.take, start: p.from, end: p.to });
  };

  return (
    <div className="takes" data-testid="takes-editor">
      <div className="pr__bar">
        <span className="panel-title">{`${track.name} · take folder (${takes.length} takes)`}</span>
        <label className="pr__check" title="Crossfade centred on each boundary between takes">
          Crossfade
          <NumberField
            label="Comp crossfade"
            value={comp?.crossfadeMs ?? 10}
            min={0}
            max={200}
            step={1}
            suffix="ms"
            disabled={!comp || comp.flattened}
            onCommit={(crossfadeMs) => void apply({ type: "setCompCrossfade", trackId: track.id, folder, crossfadeMs })}
          />
        </label>
        <div className="pr__bar-end">
          {comp?.flattened ? (
            <Button size="sm" onClick={() => void apply({ type: "unflattenComp", trackId: track.id, folder })}>
              Unflatten
            </Button>
          ) : (
            <Button size="sm" disabled={!comp} title="Consolidate the comp into ordinary regions (the takes are kept and it can be undone)" onClick={() => void apply({ type: "flattenComp", trackId: track.id, folder })}>
              Flatten comp
            </Button>
          )}
        </div>
      </div>
      <div className="takes__body" ref={bodyRef}>
        <div className="takes__row takes__row--comp">
          <span className="takes__label">{comp ? (comp.flattened ? "Comp (flattened)" : "Comp") : "Playing"}</span>
          <div className="takes__lane" style={{ width }}>
            {segments.map((s) => (
              <div key={`${s.start}-${s.take}`} className="takes__seg" style={{ left: x(s.start), width: Math.max(1, x(s.end) - x(s.start)), background: color(s.take) }} title={`Take ${s.take}: ${map.formatPosition(s.start)} – ${map.formatPosition(s.end)}`}>
                {x(s.end) - x(s.start) > 40 ? `Take ${s.take}` : ""}
              </div>
            ))}
          </div>
        </div>
        {takes.map((r) => {
          const n = r.capture!.take;
          const used = segments.filter((s) => s.take === n);
          return (
            <div key={r.id} className="takes__row" style={{ height: ROW_H }}>
              <span className="takes__label">
                <span className="takes__swatch" style={{ background: color(n) }} />
                Take {n}
                <button
                  type="button"
                  className="pr__btn"
                  disabled={comp?.flattened}
                  title={`Play the whole of take ${n} (removes the comp)`}
                  onClick={() => void apply({ type: "selectTake", trackId: track.id, folder, take: n })}
                >
                  Use
                </button>
              </span>
              <div
                className="takes__lane"
                style={{ width }}
                data-testid={`take-${n}`}
                onPointerDown={(e) => down(e, n)}
                onPointerMove={move}
                onPointerUp={up}
                onPointerCancel={() => {
                  drag.current = null;
                  setPick(null);
                }}
              >
                <RegionWaveform
                  regionId={r.id}
                  revision={`${r.start}|${r.length}|${project.transport.tempo}`}
                  fromTick={r.start}
                  toTick={r.start + r.length}
                  widthPx={Math.max(1, x(r.start + r.length) - x(r.start))}
                  leftPx={x(r.start)}
                />
                {used.map((s) => (
                  <div key={s.start} className="takes__used" style={{ left: x(s.start), width: x(s.end) - x(s.start), borderColor: color(n) }} />
                ))}
                {pick?.take === n && <div className="takes__pick" style={{ left: x(pick.from), width: Math.max(1, x(pick.to) - x(pick.from)) }} />}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
