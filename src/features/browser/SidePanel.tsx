import { useState } from "react";
import { Icon } from "../../components/Icon";
import { useChrome, type SidePanel as PanelType } from "../../state/chrome";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { TempoMap } from "../../lib/tempoMap";
import { pendingControl } from "../topbar/menuActions";
import { FileBrowser } from "./Browser";

const listTabs: [PanelType, string][] = [["events", "Evento"], ["markers", "Marcador"], ["tempo", "Tempo"], ["signature", "Compás"]];
const filters = ["Notas", "Cambio de progr.", "Inflexión de tono", "Controlador", "Aftertouch", "Aftertouch polif…", "Exclusivo sist.", "Inform. adicional"];
export function SidePanel() {
  const panel = useChrome(s => s.sidePanel);
  const setPanel = useChrome(s => s.setSidePanel);
  const project = useProject(s => s.view?.project);
  const selection = useUi(s => s.selection);
  const [fileTab, setFileTab] = useState("Proyecto");
  const [enabled, setEnabled] = useState(filters.slice(0, -1));
  const [query, setQuery] = useState("");
  const [notes, setNotes] = useState("");
  if (!panel || !project) return null;
  const lists = listTabs.some(([id]) => panel === id);
  const map = TempoMap.fromTransport(project.transport);
  const regions = project.tracks.flatMap((t, i) => t.regions.map(r => ({ ...r, track: i + 1, trackId: t.id })));
  return <aside className="right-panel" aria-label={lists ? "Editores de lista" : panel === "files" ? "Exploradores" : panel === "notes" ? "Bloc de notas" : "Explorador de loops"}>
    <div className="right-panel__tabs segmented" role="tablist">
      {lists ? listTabs.map(([id, label]) => <button role="tab" aria-selected={panel === id} className={panel === id ? "is-on" : ""} key={id} onClick={() => setPanel(id)}>{label}</button>) : panel === "files" ? ["Proyecto", "Todos los archivos"].map(t => <button role="tab" aria-selected={fileTab === t} key={t} className={fileTab === t ? "is-on" : ""} onClick={() => setFileTab(t)}>{t}</button>) : <button role="tab" aria-selected className="is-on">{panel === "notes" ? "Proyecto" : "Loops"}</button>}
      <button className="right-panel__close" aria-label="Cerrar panel" onClick={() => setPanel(null)}><Icon name="close" size={12} /></button>
    </div>
    <div className="panel-tools">{lists && <button title="Nivel superior" onClick={() => pendingControl("Nivel superior")}><Icon name="undo" size={16} /></button>}{(panel === "files" ? ["Archivo de audio", "Edición", "Visualización"] : ["Edición", "Funciones", "Visualización"]).map(label => <button key={label} onClick={() => pendingControl(label)}>{label}<Icon name="chevronDown" size={11} /></button>)}{lists && <><button className="is-blue" title="Seguir selección" onClick={() => pendingControl("Seguir selección")}><Icon name="follow" size={17} /></button><span className="toolbar-space" /><button className="is-green" title="Salida MIDI" onClick={() => pendingControl("Salida MIDI")}><Icon name="midi" size={17} /></button></>}</div>
    {panel === "events" && <><div className="event-filters">{filters.map(f => <button key={f} aria-pressed={enabled.includes(f)} className={enabled.includes(f) ? "is-blue" : ""} onClick={() => setEnabled(enabled.includes(f) ? enabled.filter(v => v !== f) : [...enabled, f])}>{f}</button>)}</div><div className="event-options"><button onClick={() => pendingControl("Insertar evento")}><Icon name="plus" size={14} /> Notas</button><span />Cuantizar: <select aria-label="Cuantizar eventos" onChange={() => pendingControl("Cuantizar eventos")}><option>Desactivado</option><option>Semicorchea</option></select><button onClick={() => pendingControl("Cuantizar eventos")}>C</button></div></>}
    {panel === "files" && fileTab === "Todos los archivos" ? <FileBrowser /> : panel === "notes" ? <textarea className="project-notepad" aria-label="Notas de la sesión" placeholder="Escribe tus notas…" value={notes} onChange={e => setNotes(e.target.value)} /> : panel === "loops" ? <div className="loop-browser"><input aria-label="Buscar loops" placeholder="Buscar loops" value={query} onChange={e => setQuery(e.target.value)} /><div className="event-filters">{["Instrumento", "Género", "Descriptores"].map(t => <button key={t} onClick={() => pendingControl(t)}>{t}</button>)}</div><p>No hay loops en la biblioteca.</p><button onClick={() => pendingControl("Obtener loops")}>Obtener más sonidos</button></div> : <>
      {panel === "files" && <input className="media-search" aria-label="Buscar audio del proyecto" value={query} onChange={e => setQuery(e.target.value)} />}
      <div className="event-table"><table><thead><tr>{(panel === "files" ? ["Nombre", "Información", "bpm", "Longitud del pasaje"] : panel === "events" ? ["L", "M", "Posición", "Nombre", "Pista", "Duración"] : ["Posición", panel === "markers" ? "Nombre" : panel === "tempo" ? "Tempo" : "Compás"]).map((c, i) => <th key={i}>{c}</th>)}</tr></thead><tbody>
        {panel === "events" && regions.filter(() => enabled.includes("Notas")).map(r => <tr key={r.id} className={selection.regionIds.includes(r.id) ? "is-selected" : ""} onClick={() => useUi.getState().selectRegions([r.id], [r.trackId])}><td /><td>{r.muted ? "M" : ""}</td><td>{map.formatPosition(r.start)}</td><td>{r.name}</td><td>{r.track}</td><td>{Math.max(1, Math.round(r.length / 3840))} 0 0 0</td></tr>)}
        {panel === "markers" && project.markers.map(m => <tr key={m.id} onClick={() => useUi.getState().setCursor(m.tick)}><td>{map.formatPosition(m.tick)}</td><td>{m.name}</td></tr>)}
        {panel === "tempo" && [{ tick: 0, bpm: project.transport.tempo }, ...project.transport.tempoChanges.filter(c => c.tick > 0)].sort((a, b) => a.tick - b.tick).map(c => <tr key={c.tick} onClick={() => useUi.getState().setCursor(c.tick)}><td>{map.formatPosition(c.tick)}</td><td>{c.bpm.toFixed(4)}</td></tr>)}
        {panel === "signature" && [{ bar: 0, timeSignature: project.transport.timeSignature }, ...project.transport.meterChanges.filter(c => c.bar > 0)].sort((a, b) => a.bar - b.bar).map(c => <tr key={c.bar} onClick={() => useUi.getState().setCursor(map.barStart(c.bar))}><td>{map.formatPosition(map.barStart(c.bar))}</td><td>{c.timeSignature.numerator}/{c.timeSignature.denominator}</td></tr>)}
        {panel === "files" && project.media.filter(m => m.name.toLowerCase().includes(query.toLowerCase())).map(m => <tr key={m.id}><td>{m.name}</td><td>{m.info ? `${m.info.sampleRate} Hz` : ""}</td><td /><td>{m.info ? `${(m.info.frames / m.info.sampleRate).toFixed(2)} s` : ""}</td></tr>)}
      </tbody></table></div>
      <footer>{panel === "events" ? `${regions.length} evento${regions.length !== 1 ? "s" : ""}` : ""}</footer>
    </>}
  </aside>;
}
