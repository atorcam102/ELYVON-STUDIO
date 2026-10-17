import { useEffect, useMemo, useRef, useState, type WheelEvent } from "react";
import { Icon } from "../../components/Icon";
import { useProject } from "../../state/project";
import { useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";
import { TempoMap } from "../../lib/tempoMap";
import { staffStep, scoreLayout, SCORE_LEFT } from "./scoreModel";
import { pendingControl } from "../topbar/menuActions";

export function EditorTools({ score = false, zoom = 50, onZoom }: { score?: boolean; zoom?: number; onZoom?: (value: number) => void }) {
  const follow = useTransport(s => s.follow);
  const layout = useUi(s => s.layout);
  const automationOpen = layout.bottomVisible && layout.bottomTab === "automation";
  return <div className="panel-tools editor-tools">
    {(score ? ["Diseño", "Edición", "Funciones", "Visualización"] : ["Edición", "Visualización"]).map(label => <button key={label} onClick={() => pendingControl(label)}>{label}<Icon name="chevronDown" size={11} /></button>)}
    {score && <><button className="is-purple" title="Vista lineal" onClick={() => pendingControl("Vista lineal")}><Icon name="tracks" size={17} /></button><button title="Vista de página" onClick={() => pendingControl("Vista de página")}><Icon name="note" size={17} /></button></>}
    <button title="Automatización" aria-pressed={automationOpen} onClick={() => useUi.getState().setLayout({ bottomVisible: true, bottomTab: "automation" })}><Icon name="automation" size={18} /></button>
    <button className={follow ? "is-blue" : ""} title="Seguir cursor" aria-pressed={follow} onClick={() => useTransport.getState().setFollow(!follow)}><Icon name="follow" size={18} /></button>
    <button className="is-gold" title="Enlazar" onClick={() => pendingControl("Enlazar editores")}><Icon name="link" size={18} /></button>
    <button title="Herramienta puntero" onClick={() => pendingControl("Herramientas de partitura")}><Icon name="pointer" size={17} /><Icon name="chevronDown" size={10} /></button>
    <span className="toolbar-space" />
    <Icon name="horizontal" size={16} /><input aria-label="Zoom del editor" type="range" value={zoom} onChange={e => onZoom ? onZoom(Number(e.target.value)) : pendingControl("Zoom del editor")} />
  </div>;
}

/**
 * Simplified, read-only notation of the selected MIDI region: pitch (with
 * sharps and ledger lines), position inside each bar and the project's
 * metre. Rhythmic note values are not engraved; a duration bar shows length.
 */
export function ScoreEditor() {
  const [zoom, setZoom] = useState(50);
  const [scroll, setScroll] = useState<{ region: string | null; bar: number }>({ region: null, bar: 0 });
  const [width, setWidth] = useState(800);
  const ref = useRef<HTMLCanvasElement>(null);
  const project = useProject(s => s.view?.project);
  const ids = useUi(s => s.selection.regionIds);
  const selected = useMemo(() => {
    const regions = project?.tracks.flatMap(t => t.regions) ?? [];
    return [...ids].reverse().map(id => regions.find(r => r.id === id)).find(r => r);
  }, [project, ids]);
  const region = selected?.content.type === "midi" ? selected : undefined;
  const map = useMemo(() => (project ? TempoMap.fromTransport(project.transport) : null), [project]);
  const layout = useMemo(() => (map ? scoreLayout(map, region ?? null, width, zoom) : null), [map, region, width, zoom]);
  const offset = scroll.region === (region?.id ?? null) ? scroll.bar : 0;
  const shownFrom = layout ? Math.min(offset, Math.max(0, layout.totalBars - layout.visible)) : 0;
  const firstAbs = (layout?.firstBar ?? 0) + shownFrom;
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const observer = new ResizeObserver(() => setWidth(canvas.getBoundingClientRect().width));
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas || !map || !layout) return;
    const { width: w, height } = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio;
    canvas.width = w * dpr; canvas.height = height * dpr;
    const g = canvas.getContext("2d")!;
    g.scale(dpr, dpr);
    g.fillStyle = "#c2c2b7"; g.fillRect(0, 0, w, height);
    const staves = [{ top: 30, clef: "𝄞", base: 30 }, { top: 120, clef: "𝄢", base: 18 }];
    const right = SCORE_LEFT + layout.visible * layout.barWidth;
    g.fillStyle = "#fffff1"; g.fillRect(0, 10, right + 6, 175);
    g.strokeStyle = "#34342e"; g.lineWidth = 1;
    const tsFirst = map.barAt(map.barStart(firstAbs)).ts;
    for (const st of staves) {
      for (let i = 0; i < 5; i++) { g.beginPath(); g.moveTo(10, st.top + i * 10 + .5); g.lineTo(right, st.top + i * 10 + .5); g.stroke(); }
      g.fillStyle = "#111"; g.font = '46px "Segoe UI Symbol", serif'; g.fillText(st.clef, 16, st.top + (st.base === 30 ? 38 : 32));
      g.font = "bold 20px Georgia, serif"; g.fillText(String(tsFirst.numerator), 112, st.top + 18); g.fillText(String(tsFirst.denominator), 112, st.top + 38);
    }
    g.beginPath(); g.moveTo(10.5, 30); g.lineTo(10.5, 160); g.stroke();
    for (let i = 0; i <= layout.visible; i++) {
      const x = Math.round(SCORE_LEFT + i * layout.barWidth) + .5;
      g.beginPath(); g.moveTo(x, 30); g.lineTo(x, 160); g.stroke();
      const ts = map.barAt(map.barStart(firstAbs + i)).ts;
      const prev = map.barAt(map.barStart(Math.max(0, firstAbs + i - 1))).ts;
      if (i > 0 && i < layout.visible && (ts.numerator !== prev.numerator || ts.denominator !== prev.denominator)) {
        g.fillStyle = "#111"; g.font = "bold 13px Georgia, serif";
        for (const st of staves) { g.fillText(String(ts.numerator), x + 3, st.top + 14); g.fillText(String(ts.denominator), x + 3, st.top + 34); }
      }
    }
    g.font = "12px \"Segoe UI\", sans-serif"; g.fillStyle = "#333";
    if (!region || region.content.type !== "midi") {
      g.fillText(selected ? "La partitura solo muestra pasajes MIDI." : "Selecciona un pasaje MIDI para ver su partitura.", SCORE_LEFT + 8, 205);
      return;
    }
    g.fillText("Vista simplificada: altura y posición en el compás; las figuras rítmicas no están disponibles.", SCORE_LEFT + 8, 205);
    const inner = layout.barWidth - 18;
    for (const n of region.content.notes) {
      const tick = region.start + n.start;
      const b = map.barAt(tick);
      const i = b.bar - firstAbs;
      if (i < 0 || i >= layout.visible) continue;
      const per = map.barStart(b.bar + 1) - b.start;
      const x = SCORE_LEFT + i * layout.barWidth + 12 + ((tick - b.start) / per) * inner;
      const { step, sharp } = staffStep(n.pitch);
      const st = n.pitch >= 60 ? staves[0]! : staves[1]!;
      const bottom = st.top + 40;
      const yOf = (l: number) => bottom - (l - st.base) * 5;
      const y = yOf(step);
      g.strokeStyle = "#34342e";
      for (let l = st.base - 2; l >= step; l -= 2) { g.beginPath(); g.moveTo(x - 8, yOf(l) + .5); g.lineTo(x + 8, yOf(l) + .5); g.stroke(); }
      for (let l = st.base + 10; l <= step; l += 2) { g.beginPath(); g.moveTo(x - 8, yOf(l) + .5); g.lineTo(x + 8, yOf(l) + .5); g.stroke(); }
      const len = Math.min(inner, (n.length / per) * inner);
      g.fillStyle = n.muted ? "rgba(120,120,110,.25)" : "rgba(40,90,170,.22)"; g.fillRect(x, y - 2, Math.max(2, len), 4);
      g.fillStyle = n.muted ? "#8a8a80" : "#111"; g.beginPath(); g.ellipse(x, y, 5, 3.6, -.35, 0, 2 * Math.PI); g.fill();
      const up = step < st.base + 4;
      g.fillRect(up ? x + 4 : x - 5, up ? y - 30 : y, 1.2, 30);
      if (sharp) { g.font = "14px \"Segoe UI Symbol\", serif"; g.fillText("♯", x - 17, y + 5); }
    }
  }, [map, layout, region, selected, firstAbs]);
  const rulerBars = layout ? Array.from({ length: layout.visible }, (_, i) => firstAbs + i + 1) : [];
  const onWheel = (e: WheelEvent) => {
    if (!layout) return;
    const max = Math.max(0, layout.totalBars - layout.visible);
    setScroll({ region: region?.id ?? null, bar: Math.max(0, Math.min(max, shownFrom + Math.sign(e.deltaY || e.deltaX))) });
  };
  return <div className="score-editor"><EditorTools score zoom={zoom} onZoom={setZoom} /><div className="score-ruler">{rulerBars.map(b => <span key={b}>{b}</span>)}</div><canvas ref={ref} aria-label="Partitura del pasaje seleccionado" onWheel={onWheel} /></div>;
}

/**
 * Tempo and metre at the cursor from the project's tempo map. Tempo analysis
 * of audio (detecting a tempo from a performance) is not available.
 */
export function SmartTempoEditor() {
  const project = useProject(s => s.view?.project);
  const ids = useUi(s => s.selection.regionIds);
  const cursor = useUi(s => s.cursorTicks);
  const region = project?.tracks.flatMap(t => t.regions).find(r => ids.includes(r.id));
  const map = project ? TempoMap.fromTransport(project.transport) : null;
  const pos = map?.position(cursor);
  const ts = map?.timeSignatureAt(cursor);
  const changes = project?.transport.tempoChanges.length ?? 0;
  return <div className="smart-tempo"><EditorTools /><div className="smart-tempo__display"><span>{pos ? `${pos.bar}. ${pos.beat}.` : "—"}<small>COMPÁS TIEMPO</small></span><span>{map?.bpmAt(cursor).toFixed(1)}<small>TEMPO</small></span><span>{changes}<small>CAMBIOS</small></span><span>{ts?.numerator}/{ts?.denominator}<small>COMPÁS</small></span></div><div className="smart-tempo__overview" /><div className="smart-tempo__ruler">{pos?.bar ?? 1}<br />{ts?.numerator}/{ts?.denominator}</div><div className="smart-tempo__region"><header>{region?.name ?? "Selecciona un pasaje"} — análisis de tempo no disponible</header><div className="smart-tempo__beats">{Array.from({ length: ts?.numerator ?? 4 }, (_, b) => <div key={b} />)}</div><div className="smart-tempo__pitches"><span>C4</span><span>C3</span></div></div></div>;
}

export function SmartControls() {
  return <div className="smart-controls"><EditorTools /><div><Icon name="sliders" size={38} /><p>Smart Controls</p><button onClick={() => pendingControl("Asignar controles")}>Asignar controles…</button></div></div>;
}
