import { Icon } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import {
  addTrack,
  deleteSelection,
  duplicateSelectedTracks,
  redo,
  undo,
} from "../project/editActions";
import {
  duplicateSelectedRegions,
  exportDiagnosticRender,
  importAudioDialog,
  setSelectedRegionsMuted,
  splitSelectedAtPlayhead,
} from "../project/regionActions";

function ToolbarMenu({
  label,
  entries,
}: {
  label: string;
  entries: MenuEntry[];
}) {
  const menu = useMenu();
  return (
    <>
      <button
        type="button"
        className="arrangement-menu"
        aria-haspopup="menu"
        aria-expanded={!!menu.at}
        onClick={(e) => menu.openBelow(e.currentTarget)}
      >
        {label}
        <Icon name="chevronDown" size={9} />
      </button>
      {menu.at && <Menu at={menu.at} entries={entries} onClose={menu.close} />}
    </>
  );
}

/** Presents existing editor commands in the compact reference toolbar. */
export function ArrangementMenus() {
  const view = useProject((s) => s.view);
  const layout = useUi((s) => s.layout);
  const setLayout = useUi((s) => s.setLayout);
  const selected = useUi((s) => s.selection.trackIds.length > 0 || s.selection.regionIds.length > 0);
  const regions = useUi((s) => s.selection.regionIds.length > 0);
  const edit: MenuEntry[] = [
    {
      id: "undo",
      label: view?.undoLabel ? `Undo ${view.undoLabel}` : "Undo",
      icon: "undo",
      disabled: !view?.undoLabel,
      onSelect: undo,
    },
    {
      id: "redo",
      label: view?.redoLabel ? `Redo ${view.redoLabel}` : "Redo",
      icon: "redo",
      disabled: !view?.redoLabel,
      onSelect: redo,
    },
    "separator",
    { id: "split", label: "Split regions at playhead", icon: "split", disabled: !regions, onSelect: () => void splitSelectedAtPlayhead() },
    { id: "dupRegions", label: "Duplicate regions", icon: "duplicate", disabled: !regions, onSelect: () => void duplicateSelectedRegions() },
    { id: "muteRegions", label: "Mute / unmute regions", icon: "muteRegion", disabled: !regions, onSelect: () => void setSelectedRegionsMuted() },
    "separator",
    {
      id: "duplicate",
      label: "Duplicate tracks",
      icon: "duplicate",
      disabled: !selected,
      onSelect: () => void duplicateSelectedTracks(),
    },
    {
      id: "delete",
      label: "Delete selection",
      icon: "trash",
      disabled: !selected,
      onSelect: () => void deleteSelection(),
    },
  ];
  const functions: MenuEntry[] = [
    { id: "import", label: "Import audio files…", icon: "importAudio", onSelect: () => void importAudioDialog() },
    { id: "render", label: "Export diagnostic render…", icon: "exportAudio", onSelect: () => void exportDiagnosticRender() },
    { id: "media", label: "Project media (repair, collect)…", onSelect: () => useUi.getState().setMediaOpen(true) },
    "separator",
    {
      id: "audio",
      label: "New audio track",
      icon: "audio",
      onSelect: () => void addTrack("audio"),
    },
    {
      id: "instrument",
      label: "New instrument track",
      icon: "instrument",
      onSelect: () => void addTrack("instrument"),
    },
    {
      id: "midi",
      label: "New MIDI track",
      icon: "midi",
      onSelect: () => void addTrack("midi"),
    },
  ];
  const panels: MenuEntry[] = [
    {
      id: "inspector",
      label: `${layout.inspectorVisible ? "Hide" : "Show"} inspector`,
      icon: "panelLeft",
      onSelect: () => setLayout({ inspectorVisible: !layout.inspectorVisible }),
    },
    {
      id: "library",
      label: `${layout.browserVisible ? "Hide" : "Show"} library`,
      icon: "panelRight",
      onSelect: () => setLayout({ browserVisible: !layout.browserVisible }),
    },
    {
      id: "mixer",
      label: "Show mixer",
      icon: "mixer",
      onSelect: () => setLayout({ bottomVisible: true, bottomTab: "mixer" }),
    },
    {
      id: "piano",
      label: "Show piano roll",
      icon: "piano",
      onSelect: () =>
        setLayout({ bottomVisible: true, bottomTab: "pianoRoll" }),
    },
  ];
  return (
    <div className="arrangement-menus">
      <ToolbarMenu label="Edit" entries={edit} />
      <ToolbarMenu label="Functions" entries={functions} />
      <ToolbarMenu label="View" entries={panels} />
      <span className="arrangement-menu-divider" />
      <span className="arrangement-view-selected" title="Tracks">
        <Icon name="tracks" size={14} />
      </span>
    </div>
  );
}
