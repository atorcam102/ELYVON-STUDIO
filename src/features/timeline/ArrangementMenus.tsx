import { Icon } from "../../components/Icon";
import { pendingControl } from "../topbar/menuActions";
import { Menu, type MenuEntry } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
import { useProject } from "../../state/project";
import { useTransport } from "../../state/transport";
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

/** Arrangement tool and ripple mode, behind the reference toolbar's tool button. */
function ToolsMenu() {
  const menu = useMenu();
  const tool = useUi((s) => s.tool);
  const ripple = useUi((s) => s.ripple);
  const entries: MenuEntry[] = [
    { id: "pointer", label: `${tool === "pointer" ? "✓ " : ""}Puntero`, icon: "pointer", onSelect: () => useUi.getState().setTool("pointer") },
    { id: "marquee", label: `${tool === "marquee" ? "✓ " : ""}Marquesina`, icon: "grid", onSelect: () => useUi.getState().setTool("marquee") },
    "separator",
    { id: "ripple", label: `${ripple ? "✓ " : ""}Edición en cadena (ripple)`, onSelect: () => useUi.getState().setRipple(!ripple) },
  ];
  return (
    <>
      <button
        type="button"
        className="arrangement-menu"
        title="Herramientas"
        aria-haspopup="menu"
        aria-expanded={!!menu.at}
        onClick={(e) => menu.openBelow(e.currentTarget)}
      >
        <Icon name={tool === "marquee" ? "grid" : "pointer"} size={16} />
        <Icon name="chevronDown" size={10} />
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
  const follow = useTransport((s) => s.follow);
  const automationOpen = layout.bottomVisible && layout.bottomTab === "automation";
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
      <button className="arrangement-menu arrangement-back" title="Volver" onClick={undo} disabled={!view?.undoLabel}><Icon name="undo" size={17} /></button>
      <ToolbarMenu label="Edición" entries={edit} />
      <ToolbarMenu label="Funciones" entries={functions} />
      <ToolbarMenu label="Visualización" entries={panels} />
      <span className="arrangement-menu-divider" />
      <button className="arrangement-menu" title="Live Loops" onClick={() => pendingControl("Live Loops")}><Icon name="grid" size={16} /></button>
      <button className="arrangement-view-selected" title="Pistas" aria-pressed="true">
        <Icon name="tracks" size={14} />
      </button>
      <div className="arrangement-extra-tools">
        <button className={`arrangement-menu${automationOpen ? " is-blue" : ""}`} title="Automatización" aria-pressed={automationOpen} onClick={() => setLayout(automationOpen ? { bottomVisible: false } : { bottomVisible: true, bottomTab: "automation" })}><Icon name="automation" size={18} /></button>
        <button className="arrangement-menu" title="Flex" onClick={() => pendingControl("Flex Pitch/Time")}><Icon name="waveform" size={18} /></button>
        <button className={`arrangement-menu${follow ? " is-blue" : ""}`} title="Seguir cursor" aria-pressed={follow} onClick={() => useTransport.getState().setFollow(!follow)}><Icon name="follow" size={17} /></button>
        <ToolsMenu />
        <button className="arrangement-menu" title="Herramienta secundaria" onClick={() => pendingControl("Herramienta secundaria")}><Icon name="plus" size={17} /><Icon name="chevronDown" size={10} /></button>
      </div>
    </div>
  );
}
