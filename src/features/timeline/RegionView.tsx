import { memo } from "react";
import type { MidiNote } from "../../bindings/MidiNote";
import type { Region } from "../../bindings/Region";
import { DemoWaveform } from "./DemoWaveform";
import { RegionWaveform } from "./RegionWaveform";

interface RegionViewProps {
  region: Region;
  color: string;
  left: number;
  width: number;
  top: number;
  height: number;
  selected: boolean;
  dimmed: boolean;
  demo?: boolean;
  /** Visual offset while dragging. */
  offset: { x: number; y: number } | null;
  invalid: boolean;
  audio?: AudioDisplay;
  onPointerDown: (
    e: React.PointerEvent<HTMLDivElement>,
    region: Region,
  ) => void;
  onTrimStart: (e: React.PointerEvent<HTMLDivElement>, region: Region, edge: "start" | "end") => void;
  onContextMenu: (e: React.MouseEvent<HTMLDivElement>, region: Region) => void;
}

/** Compact note overview drawn inside MIDI regions. */
const MidiPreview = memo(
  function MidiPreview({
    notes,
    length,
  }: {
    notes: MidiNote[];
    length: number;
  }) {
    if (notes.length === 0) return null;
    let lo = 127;
    let hi = 0;
    for (const n of notes) {
      lo = Math.min(lo, n.pitch);
      hi = Math.max(hi, n.pitch);
    }
    const span = Math.max(12, hi - lo + 1);
    const base = lo - Math.floor((span - (hi - lo + 1)) / 2);
    return (
      <svg
        className="region__notes"
        viewBox={`0 0 ${length} ${span}`}
        preserveAspectRatio="none"
        aria-hidden
      >
        {notes.map((n, i) => (
          <rect
            key={i}
            x={n.start}
            y={span - (n.pitch - base) - 1}
            width={Math.max(n.length, length / 400)}
            height={0.8}
          />
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
          return (
            n.pitch === m.pitch && n.start === m.start && n.length === m.length
          );
        }))),
);

export interface AudioDisplay {
  /** Media pool status of the region's file. */
  status: "loading" | "ready" | "failed" | "none";
  message?: string;
  /** Px from the region's left edge where the fade-in ends / fade-out starts. */
  fadeInPx: number;
  fadeOutPx: number;
  /** Visible part of the region (px from its left edge) and the ticks it spans. */
  visibleLeftPx: number;
  visibleWidthPx: number;
  fromTick: number;
  toTick: number;
  revision: string;
  looped: boolean;
}

export const RegionView = memo(function RegionView({
  region,
  color,
  left,
  width,
  top,
  height,
  selected,
  dimmed,
  demo = false,
  offset,
  invalid,
  audio,
  onPointerDown,
  onTrimStart,
  onContextMenu,
}: RegionViewProps) {
  const isMidi = region.content.type === "midi";
  const isPlaceholder = region.content.type === "audio" && region.content.media === null;
  const classes = [
    "region",
    isMidi ? "region--midi" : "region--audio",
    isPlaceholder && demo && "region--demo",
    selected && "is-selected",
    dimmed && "is-dimmed",
    region.muted && "is-muted",
    offset && "is-dragging",
    invalid && "is-invalid",
  ]
    .filter(Boolean)
    .join(" ");
  const status = audio?.status;
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
      onContextMenu={(e) => onContextMenu(e, region)}
      title={
        isPlaceholder
          ? `${region.name} — demo placeholder (no audio file)`
          : status === "failed"
            ? `${region.name} — ${audio?.message ?? "file unavailable"}`
            : `${region.name}${region.muted ? " (muted)" : ""}`
      }
      data-region-id={region.id}
    >
      <div className="region__header">
        <span className="region__name">
          {region.muted ? "◌ " : ""}
          {region.name}
          {audio?.looped ? " ⟳" : ""}
        </span>
      </div>
      <div className="region__body">
        {region.content.type === "midi" ? (
          <MidiPreview notes={region.content.notes} length={region.length} />
        ) : isPlaceholder ? (
          <div className="region__placeholder">
            {demo && <DemoWaveform id={region.id} />}
            <span>{demo ? "Demo preview · No audio file" : "No audio file"}</span>
          </div>
        ) : status === "failed" ? (
          <div className="region__placeholder region__placeholder--error">
            <span>{audio?.message ?? "File unavailable"}</span>
          </div>
        ) : status === "loading" ? (
          <div className="region__placeholder">
            <span>Loading audio…</span>
          </div>
        ) : audio && audio.visibleWidthPx > 0 ? (
          <>
            <RegionWaveform
              regionId={region.id}
              revision={audio.revision}
              fromTick={audio.fromTick}
              toTick={audio.toTick}
              widthPx={audio.visibleWidthPx}
              leftPx={audio.visibleLeftPx}
            />
            {(audio.fadeInPx > 1 || audio.fadeOutPx < width - 1) && (
              <svg className="region__fades" viewBox={`0 0 ${Math.max(2, width)} 100`} preserveAspectRatio="none" aria-hidden>
                {audio.fadeInPx > 1 && <polyline points={`0,100 ${audio.fadeInPx},0`} vectorEffect="non-scaling-stroke" />}
                {audio.fadeOutPx < width - 1 && <polyline points={`${audio.fadeOutPx},0 ${width},100`} vectorEffect="non-scaling-stroke" />}
              </svg>
            )}
          </>
        ) : null}
      </div>
      {!offset && (
        <>
          <div className="region__trim region__trim--start" onPointerDown={(e) => onTrimStart(e, region, "start")} title="Drag to trim the start" />
          <div className="region__trim region__trim--end" onPointerDown={(e) => onTrimStart(e, region, "end")} title="Drag to trim the end" />
        </>
      )}
    </div>
  );
});
