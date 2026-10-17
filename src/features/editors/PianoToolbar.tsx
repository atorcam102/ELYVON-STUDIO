import { Icon } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
import { useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";
import { pendingControl } from "../topbar/menuActions";

type PianoTool = "pointer" | "pencil" | "eraser";

interface PianoToolbarProps {
  advanced: boolean;
  onAdvanced: () => void;
  showLane: boolean;
  onLane: () => void;
  onFunctions: (at: { x: number; y: number }) => void;
  audition: boolean;
  onAudition: () => void;
  snap: boolean;
  onSnap: () => void;
  tool: PianoTool;
  onTool: (tool: PianoTool) => void;
  zoom: number;
  onZoom: (value: number) => void;
  keyHeight: number;
  onKeyHeight: (value: number) => void;
}

/** Presentation only: editing and zoom values remain owned by PianoRoll. */
export function PianoToolbar(props: PianoToolbarProps) {
  const tools = useMenu();
  const display = useMenu();
  const follow = useTransport(s => s.follow);
  const labels: Record<PianoTool, string> = { pointer: "Puntero", pencil: "Lápiz", eraser: "Borrador" };
  const entries: MenuEntry[] = (["pointer", "pencil", "eraser"] as const).map((tool, index) => ({
    id: tool, label: `${props.tool === tool ? "✓ " : ""}${labels[tool]}`, icon: tool,
    shortcut: String(index + 1), onSelect: () => props.onTool(tool),
  }));
  const viewEntries: MenuEntry[] = [
    { id: "lane", label: `${props.showLane ? "Ocultar" : "Mostrar"} controles MIDI`, onSelect: props.onLane },
    { id: "advanced", label: `${props.advanced ? "Ocultar" : "Mostrar"} herramientas de edición`, onSelect: props.onAdvanced },
    "separator",
    { id: "snap", label: `${props.snap ? "✓ " : ""}Ajustar a parrilla`, onSelect: props.onSnap },
  ];
  return <div className="panel-tools piano-tools">
    <div className="editor-menu-group">
      <button onClick={props.onAdvanced} aria-pressed={props.advanced}>Edición<Icon name="chevronDown" size={10} /></button>
      <button onClick={e => { const r = e.currentTarget.getBoundingClientRect(); props.onFunctions({ x: r.left, y: r.bottom + 4 }); }}>Funciones<Icon name="chevronDown" size={10} /></button>
      <button aria-haspopup="menu" aria-expanded={!!display.at} onClick={e => display.openBelow(e.currentTarget)}>Visualización<Icon name="chevronDown" size={10} /></button>
    </div>
    <div className="editor-button-group">
      <button title="Mostrar controles MIDI" aria-label="Mostrar controles MIDI" aria-pressed={props.showLane} onClick={props.onLane}><Icon name="collapseVertical" size={18} /></button>
      <button title="Automatización" aria-label="Automatización del editor" onClick={() => useUi.getState().setLayout({ bottomTab: "automation" })}><Icon name="automation" size={19} /></button>
    </div>
    <div className="editor-button-group editor-audition-group">
      <button title="Audición de notas seleccionadas" aria-label="Audición de notas seleccionadas" onClick={() => pendingControl("Audición de notas seleccionadas")}><Icon name="audition" size={18} /></button>
      <button title="Escuchar notas" aria-label="Escuchar notas" aria-pressed={props.audition} className={props.audition ? "is-green" : ""} onClick={props.onAudition}><Icon name="audition" size={18} /></button>
    </div>
    <div className="editor-button-group">
      <button title="Seguir cursor" aria-label="Seguir cursor en el editor" aria-pressed={follow} className={follow ? "is-blue" : ""} onClick={() => useTransport.getState().setFollow(!follow)}><Icon name="catchPlayhead" size={19} /></button>
      <button title="Enlazar selección" aria-label="Enlazar selección" className="is-gold" onClick={() => pendingControl("Enlazar selección del editor")}><Icon name="link" size={19} /></button>
    </div>
    <div className="editor-button-group editor-tool-group">
      <button title={`Herramienta: ${labels[props.tool]}`} aria-label="Herramientas del editor" aria-haspopup="menu" aria-expanded={!!tools.at} onClick={e => tools.openBelow(e.currentTarget)}><Icon name={props.tool} size={18} /><Icon name="chevronDown" size={10} /></button>
      <button title="Lápiz (2)" aria-label="Lápiz (2)" aria-pressed={props.tool === "pencil"} onClick={() => props.onTool("pencil")}><Icon name="pencil" size={18} /></button>
    </div>
    <span className="toolbar-space" />
    <button title="Ajustar a parrilla" aria-label="Ajustar a parrilla" aria-pressed={props.snap} className={`editor-snap${props.snap ? " is-blue" : ""}`} onClick={props.onSnap}><Icon name="legato" size={18} /></button>
    <label className="editor-zoom editor-zoom--vertical" title="Zoom vertical"><Icon name="vertical" size={15} /><input aria-label="Zoom vertical del teclado" type="range" min={8} max={28} value={props.keyHeight} onChange={e => props.onKeyHeight(Number(e.target.value))} /></label>
    <label className="editor-zoom" title="Zoom horizontal"><Icon name="horizontal" size={15} /><input aria-label="Zoom del editor de teclado" type="range" min={.03} max={.5} step={.01} value={props.zoom} onChange={e => props.onZoom(Number(e.target.value))} /></label>
    {tools.at && <Menu at={tools.at} entries={entries} onClose={tools.close} />}
    {display.at && <Menu at={display.at} entries={viewEntries} onClose={display.close} />}
  </div>;
}
