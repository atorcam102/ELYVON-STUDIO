import { useMemo } from "react";
import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import { Badge } from "../../components/Button";
import { Icon } from "../../components/Icon";
import { gridBackground } from "../../lib/gridBackground";
import { formatBarBeat, noteName, ticksPerBar, ticksPerBeat } from "../../lib/music";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";

const KEY_HEIGHT = 13;
const PX_PER_TICK = 0.06;

function isBlack(pitch: number) {
  return [1, 3, 6, 8, 10].includes(pitch % 12);
}

function selectedRegion(project: Project | undefined, ids: string[], type: "midi" | "audio"): Region | undefined {
  if (!project) return undefined;
  const all = project.tracks.flatMap((t) => t.regions);
  for (let i = ids.length - 1; i >= 0; i--) {
    const r = all.find((x) => x.id === ids[i]);
    if (r?.content.type === type) return r;
  }
  return undefined;
}

/**
 * Read-only view of the notes in the selected MIDI region. Note editing tools
 * are scheduled for a later phase and are intentionally not shown here.
 */
export function PianoRoll() {
  const project = useProject((s) => s.view?.project);
  const regionIds = useUi((s) => s.selection.regionIds);
  const region = selectedRegion(project, regionIds, "midi");

  const range = useMemo(() => {
    if (region?.content.type !== "midi" || region.content.notes.length === 0) return { lo: 48, hi: 84 };
    let lo = 127;
    let hi = 0;
    for (const n of region.content.notes) {
      lo = Math.min(lo, n.pitch);
      hi = Math.max(hi, n.pitch);
    }
    return { lo: Math.max(0, lo - 5), hi: Math.min(127, hi + 5) };
  }, [region]);

  if (!project || !region || region.content.type !== "midi") {
    return (
      <div className="editor-empty">
        <Icon name="piano" size={22} />
        <p>Select a MIDI region to view its notes.</p>
        <span>The piano roll is a read-only preview in this version; note editing arrives in a later phase.</span>
      </div>
    );
  }

  const ts = project.transport.timeSignature;
  const pitches: number[] = [];
  for (let p = range.hi; p >= range.lo; p--) pitches.push(p);
  const width = region.length * PX_PER_TICK;
  const height = pitches.length * KEY_HEIGHT;
  const barPx = ticksPerBar(ts) * PX_PER_TICK;
  const beatPx = ticksPerBeat(ts) * PX_PER_TICK;

  return (
    <div className="piano-roll">
      <div className="piano-roll__bar">
        <span className="panel-title">{region.name}</span>
        <span className="hint">
          {formatBarBeat(region.start, ts)} · {region.content.notes.length} notes
        </span>
        <Badge tone="warning">Read-only preview</Badge>
      </div>
      <div className="piano-roll__scroll">
        <div className="piano-roll__keys" style={{ height }}>
          {pitches.map((p) => (
            <div key={p} className={`pr-key${isBlack(p) ? " is-black" : ""}`} style={{ height: KEY_HEIGHT }}>
              {p % 12 === 0 && <span>{noteName(p)}</span>}
            </div>
          ))}
        </div>
        <div
          className="piano-roll__grid"
          style={{
            width,
            height,
            ...gridBackground([
              { axis: "x", every: barPx, color: "var(--grid-bar)" },
              { axis: "x", every: beatPx, color: "var(--grid-beat)" },
              { axis: "y", every: KEY_HEIGHT, color: "var(--grid-row)" },
            ]),
          }}
        >
          {region.content.notes.map((n, i) => (
            <div
              key={i}
              className="pr-note"
              title={`${noteName(n.pitch)} · velocity ${n.velocity}`}
              style={{
                left: n.start * PX_PER_TICK,
                width: Math.max(2, n.length * PX_PER_TICK - 1),
                top: (range.hi - n.pitch) * KEY_HEIGHT + 1,
                height: KEY_HEIGHT - 2,
                opacity: 0.55 + (n.velocity / 127) * 0.45,
              }}
            >
              {n.length * PX_PER_TICK > 26 && <span>{noteName(n.pitch)}</span>}
            </div>
          ))}
        </div>
      </div>
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
            {region.name}: {region.content.source ? region.content.source.path : "placeholder region without an audio file"}
          </p>
          <span>Waveform display and audio editing need the audio engine and file import (Phase 2).</span>
        </>
      ) : (
        <>
          <p>Audio editor</p>
          <span>Waveform display and audio editing need the audio engine and file import (Phase 2).</span>
        </>
      )}
    </div>
  );
}
