import { memo, useState } from "react";
import type { Track } from "../../bindings/Track";
import { InlineEdit } from "../../components/Fields";
import { Icon } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
import { dbToFaderPos, faderPosToDb, formatDb, MIN_DB } from "../../lib/format";
import { shortcutLabel } from "../../lib/platform";
import { useProject } from "../../state/project";
import { KIND_ICON, KIND_LABEL, TRACK_COLORS } from "./trackMeta";

interface TrackHeaderProps {
  track: Track;
  index: number;
  selected: boolean;
  audible: boolean;
  height: number;
  onSelect: (id: string, e: React.MouseEvent | React.PointerEvent) => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onReorderStart: (e: React.PointerEvent<HTMLDivElement>, id: string) => void;
}

export const TrackHeader = memo(function TrackHeader({
  track,
  index,
  selected,
  audible,
  height,
  onSelect,
  onDuplicate,
  onDelete,
  onReorderStart,
}: TrackHeaderProps) {
  const apply = useProject((s) => s.apply);
  const [renaming, setRenaming] = useState(false);
  const menu = useMenu();

  const entries: MenuEntry[] = [
    { id: "rename", label: "Rename", icon: "edit", onSelect: () => setRenaming(true) },
    { id: "duplicate", label: "Duplicate", icon: "duplicate", shortcut: shortcutLabel("Mod+D"), onSelect: onDuplicate },
    { id: "delete", label: "Delete", icon: "trash", danger: true, shortcut: "Del", onSelect: onDelete },
    "separator",
    { heading: "Colour" },
    {
      key: "colors",
      custom: (
        <div className="swatches">
          {TRACK_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              className={`swatch${c === track.color ? " is-active" : ""}`}
              style={{ background: c }}
              aria-label={`Colour ${c}`}
              onClick={() => {
                menu.close();
                void apply({ type: "setTrackColor", id: track.id, color: c });
              }}
            />
          ))}
        </div>
      ),
    },
  ];

  return (
    <div
      className={`track-header${selected ? " is-selected" : ""}${audible ? "" : " is-silent"}`}
      style={{ height, ["--track-color" as string]: track.color }}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        onSelect(track.id, e);
        onReorderStart(e, track.id);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        if (!selected) onSelect(track.id, e);
        menu.openAt(e.clientX, e.clientY);
      }}
      role="row"
      aria-selected={selected}
      aria-label={`Track ${index + 1}: ${track.name}`}
    >
      <div className="track-header__index">{index + 1}</div>
      <div className="track-header__icon" title={KIND_LABEL[track.kind]}>
        <Icon name={KIND_ICON[track.kind]} size={18} />
      </div>
      <div className="track-header__main">
        <div className="track-header__top">
          <InlineEdit
            value={track.name}
            label="Track name"
            editing={renaming}
            onEditingChange={setRenaming}
            onCommit={(name) => void apply({ type: "renameTrack", id: track.id, name })}
            className="track-header__name"
          />
        </div>
        <div className="track-header__controls" onPointerDown={(e) => e.stopPropagation()}>
          <button
            type="button"
            className={`ms-btn ms-btn--mute${track.muted ? " is-on" : ""}`}
            aria-pressed={track.muted}
            title={track.muted ? "Unmute" : "Mute"}
            onClick={() => void apply({ type: "setTracksMuted", ids: [track.id], muted: !track.muted })}
          >
            M
          </button>
          <button
            type="button"
            className={`ms-btn ms-btn--solo${track.soloed ? " is-on" : ""}`}
            aria-pressed={track.soloed}
            title={track.soloed ? "Unsolo" : "Solo"}
            onClick={() => void apply({ type: "setTracksSoloed", ids: [track.id], soloed: !track.soloed })}
          >
            S
          </button>
            <button type="button" className={`ms-btn ms-btn--rec${track.io.armed ? " is-on" : ""}`} aria-label="Activar grabación" aria-pressed={track.io.armed} onClick={() => void apply({ type: "setTrackIo", id: track.id, io: { ...track.io, armed: !track.io.armed } })}>
              R
            </button>
            <button type="button" className={`ms-btn ms-btn--input${track.io.monitor ? " is-on" : ""}`} aria-label="Monitorización de entrada" aria-pressed={track.io.monitor} onClick={() => void apply({ type: "setTrackIo", id: track.id, io: { ...track.io, monitor: !track.io.monitor } })}>I</button>
          <MiniFader
            value={track.volumeDb}
            label={`${track.name} volume`}
            onCommit={(volumeDb) => void apply({ type: "setTrackVolume", id: track.id, volumeDb })}
          />
          <input className="track-pan" type="range" aria-label={`${track.name} panorama`} min={-1} max={1} step={.01} value={track.pan} onChange={e => void apply({ type: "setTrackPan", id: track.id, pan: Number(e.target.value) })} onDoubleClick={() => void apply({ type: "setTrackPan", id: track.id, pan: 0 })} />
        </div>
      </div>
      {menu.at && <Menu at={menu.at} entries={entries} onClose={menu.close} />}
    </div>
  );
});

/** Compact horizontal volume slider in the track header (one undo step per gesture). */
function MiniFader({ value, label, onCommit }: { value: number; label: string; onCommit: (db: number) => void }) {
  const [preview, setPreview] = useState<number | null>(null);
  const db = preview ?? value;
  const commit = () => {
    if (preview !== null && preview !== value) onCommit(preview);
    setPreview(null);
  };
  return (
    <input
      type="range"
      className="mini-fader"
      aria-label={label}
      title={`${label}: ${formatDb(db)} dB`}
      min={0}
      max={1000}
      value={Math.round(dbToFaderPos(db) * 1000)}
      style={{ ["--pos" as string]: `${dbToFaderPos(db) * 100}%` }}
      onChange={(e) => setPreview(Math.max(MIN_DB, Math.round(faderPosToDb(Number(e.target.value) / 1000) * 10) / 10))}
      onPointerUp={commit}
      onKeyUp={commit}
      onBlur={commit}
      onDoubleClick={() => onCommit(0)}
    />
  );
}
