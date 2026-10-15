import { memo } from "react";
import type { MidiNote } from "../../bindings/MidiNote";
import type { Region } from "../../bindings/Region";

interface RegionViewProps {
  region: Region;
  color: string;
  left: number;
  width: number;
  top: number;
  height: number;
  selected: boolean;
  dimmed: boolean;
  /** Visual offset while dragging. */
  offset: { x: number; y: number } | null;
  invalid: boolean;
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>, region: Region) => void;
}

/** Compact note overview drawn inside MIDI regions. */
const MidiPreview = memo(
  function MidiPreview({ notes, length }: { notes: MidiNote[]; length: number }) {
    if (notes.length === 0) return <div className="region__empty">Empty</div>;
    let lo = 127;
    let hi = 0;
    for (const n of notes) {
      lo = Math.min(lo, n.pitch);
      hi = Math.max(hi, n.pitch);
    }
    const span = Math.max(12, hi - lo + 1);
    const base = lo - Math.floor((span - (hi - lo + 1)) / 2);
    return (
      <svg className="region__notes" viewBox={`0 0 ${length} ${span}`} preserveAspectRatio="none" aria-hidden>
        {notes.map((n, i) => (
          <rect key={i} x={n.start} y={span - (n.pitch - base) - 1} width={Math.max(n.length, length / 400)} height={0.8} />
        ))}
      </svg>
    );
  },
  (a, b) =>
    a.length === b.length &&
    (a.notes === b.notes ||
      (a.notes.length === b.notes.length &&
        a.notes.every((n, i) => {
          const m = b.notes[i]!;
          return n.pitch === m.pitch && n.start === m.start && n.length === m.length;
        }))),
);

export const RegionView = memo(function RegionView({
  region,
  color,
  left,
  width,
  top,
  height,
  selected,
  dimmed,
  offset,
  invalid,
  onPointerDown,
}: RegionViewProps) {
  const isMidi = region.content.type === "midi";
  const isPlaceholder = region.content.type === "audio" && region.content.source === null;
  const classes = [
    "region",
    isMidi ? "region--midi" : "region--audio",
    selected && "is-selected",
    dimmed && "is-dimmed",
    offset && "is-dragging",
    invalid && "is-invalid",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <div
      className={classes}
      style={{
        left,
        top,
        width: Math.max(2, width),
        height,
        ["--region-color" as string]: color,
        transform: offset ? `translate(${offset.x}px, ${offset.y}px)` : undefined,
      }}
      onPointerDown={(e) => onPointerDown(e, region)}
      title={isPlaceholder ? `${region.name} — demo placeholder (no audio file)` : region.name}
      data-region-id={region.id}
    >
      <div className="region__header">
        <span className="region__name">{region.name}</span>
      </div>
      <div className="region__body">
        {region.content.type === "midi" ? (
          <MidiPreview notes={region.content.notes} length={region.length} />
        ) : isPlaceholder ? (
          // Deliberately not a fake waveform: there is no audio behind this region.
          <div className="region__placeholder">
            <span>No audio file</span>
          </div>
        ) : (
          <div className="region__placeholder">
            <span>{region.content.type === "audio" ? region.content.source?.path : ""}</span>
          </div>
        )}
      </div>
    </div>
  );
});
