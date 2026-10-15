import { memo, useState } from "react";
import type { Track } from "../../bindings/Track";
import { InlineEdit } from "../../components/Fields";
import { Icon } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
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
      <div className="track-header__color" />
      <div className="track-header__index">{index + 1}</div>
      <Icon name={KIND_ICON[track.kind]} size={15} className="track-header__kind" title={KIND_LABEL[track.kind]} />
      <div className="track-header__main">
        <InlineEdit
          value={track.name}
          label="Track name"
          editing={renaming}
          onEditingChange={setRenaming}
          onCommit={(name) => void apply({ type: "renameTrack", id: track.id, name })}
          className="track-header__name"
        />
        <span className="track-header__meta">
          {KIND_LABEL[track.kind]}
          {track.regions.length > 0 && ` · ${track.regions.length} ${track.regions.length === 1 ? "region" : "regions"}`}
        </span>
      </div>
      <div className="track-header__buttons" onPointerDown={(e) => e.stopPropagation()}>
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
      </div>
      {menu.at && <Menu at={menu.at} entries={entries} onClose={menu.close} />}
    </div>
  );
});
