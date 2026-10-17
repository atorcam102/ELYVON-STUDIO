import { useState } from "react";
import { Icon } from "../../components/Icon";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { pendingControl } from "../topbar/menuActions";

const categories = ["Electronic Drums", "Acoustic Piano", "Mallet", "Synthesizer"];
export function Library() {
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const trackId = useUi(s => s.selection.trackIds[0]);
  const track = useProject(s => s.view?.project.tracks.find(t => t.id === trackId) ?? s.view?.project.tracks[0]);
  return <aside className="sound-library" aria-label="Biblioteca">
    <div className="sound-library__heading"><strong>Biblioteca</strong><button onClick={() => setCategory(null)}>Todos los sonidos <Icon name="chevronDown" size={12} /></button></div>
    <div className="sound-library__patch">{track?.name ?? "Todos los sonidos"}</div>
    <label className="sound-library__search"><Icon name="search" size={15} /><input aria-label="Buscar sonidos" placeholder="Buscar sonidos" value={search} onChange={e => setSearch(e.target.value)} /></label>
    <div className="sound-library__categories">
      {category && <button onClick={() => setCategory(null)}><Icon name="chevronUp" size={12} />{category}</button>}
      {(category ? ["Default", "Bright", "Classic", "Soft"] : categories).filter(c => c.toLowerCase().includes(search.toLowerCase())).map(c => <button key={c} onClick={() => category ? pendingControl(`${category}: ${c}`) : setCategory(c)}>{c}<Icon name="chevronRight" size={13} /></button>)}
    </div>
    <div className="sound-library__download"><button onClick={() => pendingControl("Obtener más sonidos")}><Icon name="folder" size={14} />Obtener más sonidos</button></div>
    <footer><button aria-label="Opciones de biblioteca" onClick={() => pendingControl("Opciones de biblioteca")}><Icon name="more" size={16} /><Icon name="chevronDown" size={10} /></button><button onClick={() => pendingControl("Restaurar sonido")}>Restaurar</button><span /><button onClick={() => pendingControl("Eliminar sonido")}>Eliminar</button><button onClick={() => pendingControl("Guardar sonido")}>Guardar…</button></footer>
  </aside>;
}
