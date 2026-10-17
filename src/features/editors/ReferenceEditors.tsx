import { useEffect, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { pendingControl } from "../topbar/menuActions";

export function EditorTools({ score = false, zoom = 50, onZoom }: { score?: boolean; zoom?: number; onZoom?: (value: number) => void }) {
  return <div className="panel-tools editor-tools">
    {(score ? ["Diseño", "Edición", "Funciones", "Visualización"] : ["Edición", "Visualización"]).map(label => <button key={label} onClick={() => pendingControl(label)}>{label}<Icon name="chevronDown" size={11} /></button>)}
    {score && <><button className="is-purple" title="Vista lineal" onClick={() => pendingControl("Vista lineal")}><Icon name="tracks" size={17} /></button><button title="Vista de página" onClick={() => pendingControl("Vista de página")}><Icon name="note" size={17} /></button></>}
    <button title="Automatización" onClick={() => pendingControl("Automatización")}><Icon name="automation" size={18} /></button>
    <button className="is-blue" title="Seguir cursor" onClick={() => pendingControl("Seguir cursor")}><Icon name="follow" size={18} /></button>
    <button className="is-gold" title="Enlazar" onClick={() => pendingControl("Enlazar editores")}><Icon name="link" size={18} /></button>
    <button title="Herramienta puntero" onClick={() => pendingControl("Herramientas de partitura")}><Icon name="pointer" size={17} /><Icon name="chevronDown" size={10} /></button>
    <span className="toolbar-space" />
    <Icon name="horizontal" size={16} /><input aria-label="Zoom del editor" type="range" value={zoom} onChange={e => onZoom ? onZoom(Number(e.target.value)) : pendingControl("Zoom del editor")} />
  </div>;
}

export function ScoreEditor() {
  const [zoom, setZoom] = useState(50);
  const ref = useRef<HTMLCanvasElement>(null);
  const project = useProject(s => s.view?.project);
  const ids = useUi(s => s.selection.regionIds);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas || !project) return;
    const draw = () => {
      const { width, height } = canvas.getBoundingClientRect();
      const dpr = window.devicePixelRatio;
      canvas.width = width * dpr; canvas.height = height * dpr;
      const g = canvas.getContext("2d")!;
      g.scale(dpr, dpr);
      g.fillStyle = "#c2c2b7"; g.fillRect(0, 0, width, height);
      g.scale(.6 + zoom / 125, .6 + zoom / 125);
      g.fillStyle = "#fffff1"; g.beginPath(); g.roundRect(152, 2, 76, 232, 9); g.fill();
      for (const y of [42, 152]) {
        g.strokeStyle = "#34342e"; g.lineWidth = 1;
        for (let i = 0; i < 5; i++) { g.beginPath(); g.moveTo(24, y + i * 11); g.lineTo(228, y + i * 11); g.stroke(); }
        g.beginPath(); g.moveTo(24, y); g.lineTo(24, y + 44); g.stroke();
        g.fillStyle = "#111"; g.font = '60px "Segoe UI Symbol", serif'; g.fillText(y === 42 ? "𝄞" : "𝄢", 30, y + 45);
        g.font = "bold 29px Georgia, serif"; g.fillText("4", 91, y + 22); g.fillText("4", 91, y + 44);
        g.fillRect(185, y + 11, 17, 6);
      }
      g.font = "bold 20px Georgia, serif"; g.fillText("1", 145, 19);
      const region = project.tracks.flatMap(t => t.regions).find(r => ids.includes(r.id));
      if (region?.content.type === "midi" && region.content.notes.length) {
        g.fillStyle = "#fffff1"; g.fillRect(180, 52, 25, 9);
        region.content.notes.slice(0, 16).forEach(n => {
          const x = 168 + (n.start / Math.max(1, region.length)) * 53;
          const y = 75 - (n.pitch - 60) * 3;
          g.fillStyle = "#111"; g.beginPath(); g.ellipse(x, y, 5, 3, -.3, 0, 2 * Math.PI); g.fill(); g.fillRect(x + 4, y - 24, 1, 24);
        });
      }
    };
    const observer = new ResizeObserver(draw); observer.observe(canvas); draw();
    return () => observer.disconnect();
  }, [project, ids, zoom]);
  return <div className="score-editor"><EditorTools score zoom={zoom} onZoom={setZoom} /><div className="score-ruler">{Array.from({ length: 10 }, (_, i) => <span key={i}>{i * 2 + 1}</span>)}</div><canvas ref={ref} aria-label="Partitura del pasaje seleccionado" /></div>;
}

export function SmartTempoEditor() {
  const project = useProject(s => s.view?.project);
  const ids = useUi(s => s.selection.regionIds);
  const region = project?.tracks.flatMap(t => t.regions).find(r => ids.includes(r.id));
  return <div className="smart-tempo"><EditorTools /><div className="smart-tempo__display"><span>1. 1.<small>COMPÁS TIEMPO</small></span><span>{project?.transport.tempo.toFixed(1)}<small>TEMPO</small></span><span>×2<br />/2</span><span>{project?.transport.timeSignature.numerator}/{project?.transport.timeSignature.denominator}<small>COMPÁS</small></span></div><div className="smart-tempo__overview" /><div className="smart-tempo__ruler">1<br />{project?.transport.timeSignature.numerator}/{project?.transport.timeSignature.denominator}</div><div className="smart-tempo__region"><header>{region?.name ?? "Selecciona un pasaje"}</header><div className="smart-tempo__beats">{[1, 2, 3, 4].map(b => <div key={b} />)}</div><div className="smart-tempo__pitches"><span>C4</span><span>C3</span></div></div></div>;
}

export function SmartControls() {
  return <div className="smart-controls"><EditorTools /><div><Icon name="sliders" size={38} /><p>Smart Controls</p><button onClick={() => pendingControl("Asignar controles")}>Asignar controles…</button></div></div>;
}
