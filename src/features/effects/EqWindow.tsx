import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { EffectInfo } from "../../bindings/EffectInfo";
import type { EffectParamInfo } from "../../bindings/EffectParamInfo";
import type { EffectSlot } from "../../bindings/EffectSlot";
import type { EqResponse } from "../../bindings/EqResponse";
import type { PresetEntry } from "../../bindings/PresetEntry";
import type { Spectrum } from "../../bindings/Spectrum";
import { bandParamVisible, clampParam, effectsIpc, eqBands, formatParam, fromPosition, paramValue, toPosition } from "../../lib/effects";
import { notify, notifyError } from "../../state/notifications";

/** Band colours (original palette). */
const BAND_COLORS = ["#8fa9c2", "#f0a23a", "#e3db3c", "#4fd36a", "#26cfae", "#5cb8f0", "#b886f2", "#8fa9c2"];

/** Session clipboard for "Copiar"/"Pegar" between equaliser windows. */
let clipboard: Record<string, number> | null = null;

const SCALES = [6, 12, 15, 24, 30];
const LEFT = 34;
const RIGHT = 58;
const TOP = 10;
const BOTTOM = 10;

const key = (b: number, name: string) => `b${b + 1}${name}`;

/** Small drawing of a band's shape (cuts, shelves, bell, band-pass, notch). */
function Glyph({ type, color }: { type: number; color: string }) {
  const paths: Record<number, string> = {
    0: "M2 13 C8 13 9 4 14 4 L30 4",
    1: "M2 4 L10 4 C15 4 16 11 21 11 L30 11",
    2: "M2 9 L10 9 C13 9 14 3 16 3 C18 3 19 9 22 9 L30 9 M2 9 L10 9 C13 9 14 15 16 15 C18 15 19 9 22 9",
    3: "M2 11 L11 11 C16 11 17 4 22 4 L30 4",
    4: "M2 4 L18 4 C23 4 24 13 30 13",
    5: "M2 14 C10 14 12 3 16 3 C20 3 22 14 30 14",
    6: "M2 4 L12 4 C14 4 15 14 16 14 C17 14 18 4 20 4 L30 4",
  };
  return (
    <svg width="32" height="18" viewBox="0 0 32 18" aria-hidden>
      <path d={paths[type] ?? paths[2]} fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

/**
 * A number edited by dragging vertically (Shift: fine), double-click to type,
 * committed once per gesture.
 */
function DragValue({ p, value, onCommit, color, title, dim }: { p: EffectParamInfo; value: number; onCommit: (v: number) => void; color: string; title: string; dim?: boolean }) {
  const [preview, setPreview] = useState<number | null>(null);
  const [text, setText] = useState<string | null>(null);
  const drag = useRef<{ y: number; pos: number; moved: boolean } | null>(null);
  const shown = preview ?? value;
  if (text !== null) {
    const done = (save: boolean) => {
      const v = Number(text.replace(",", ".").replace(/[^\d.+-]/g, ""));
      setText(null);
      if (save && Number.isFinite(v)) onCommit(clampParam(p, p.unit === "hz" && /k/i.test(text) ? v * 1000 : v));
    };
    return (
      <input
        className="peq-value__input"
        autoFocus
        aria-label={title}
        value={text}
        onFocus={(e) => e.target.select()}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => done(true)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") done(true);
          if (e.key === "Escape") done(false);
        }}
      />
    );
  }
  const step = (dir: number, fine: boolean) => {
    if (p.scale === "choice" || p.scale === "toggle") return clampParam(p, shown + dir);
    return fromPosition(p, toPosition(p, shown) + dir * (fine ? 0.002 : 0.01));
  };
  return (
    <button
      type="button"
      className={`peq-value${dim ? " is-dim" : ""}`}
      style={{ color: dim ? undefined : color }}
      title={`${title} — arrastra arriba/abajo (Mayús: fino), doble clic para escribir`}
      aria-label={`${title}: ${formatParam(p, shown)}`}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { y: e.clientY, pos: toPosition(p, value), moved: false };
      }}
      onPointerMove={(e) => {
        const d = drag.current;
        if (!d) return;
        const dy = d.y - e.clientY;
        if (Math.abs(dy) > 2) d.moved = true;
        if (!d.moved) return;
        if (p.scale === "choice") {
          setPreview(clampParam(p, Math.round(toPosition(p, value) * p.max + dy / 20)));
        } else {
          setPreview(fromPosition(p, d.pos + (dy / 200) * (e.shiftKey ? 0.2 : 1)));
        }
      }}
      onPointerUp={() => {
        const d = drag.current;
        drag.current = null;
        if (d?.moved && preview !== null && preview !== value) onCommit(preview);
        setPreview(null);
      }}
      onDoubleClick={() => setText(formatParam(p, value).replace(/\s?(Hz|kHz|dB|dB\/oct)$/, "").trim())}
      onKeyDown={(e) => {
        if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          onCommit(step(e.key === "ArrowUp" ? 1 : -1, e.shiftKey));
        }
      }}
    >
      {formatParam(p, shown)}
    </button>
  );
}

interface EqWindowProps {
  info: EffectInfo;
  slot: EffectSlot;
  bypass: boolean;
  commit: (next: EffectSlot, bypass?: boolean) => void;
  presets: PresetEntry[];
  setPresets: (p: PresetEntry[]) => void;
  spectrum: Spectrum | null;
  analyzer: boolean;
  setAnalyzer: (on: boolean) => void;
  latency: string;
}

/**
 * The parametric equaliser window: band shapes on top, the response of every
 * band and of the whole equaliser (computed by the core from the engine's own
 * sections) over the measured spectrum, editable values per band below.
 */
export function EqWindow({ info, slot, bypass, commit, presets, setPresets, spectrum, analyzer, setAnalyzer, latency }: EqWindowProps) {
  const bands = useMemo(() => eqBands(info), [info]);
  const output = info.params.find((p) => p.key === "outputDb")!;
  const processing = info.params.find((p) => p.key === "processing");
  const [selected, setSelected] = useState(3);
  const [scale, setScale] = useState(15);
  const [drag, setDrag] = useState<Record<string, number> | null>(null);
  const [curve, setCurve] = useState<EqResponse | null>(null);
  const [saveName, setSaveName] = useState<string | null>(null);
  const [history, setHistory] = useState<{ undo: Record<string, number>[]; redo: Record<string, number>[] }>({ undo: [], redo: [] });
  const [, bump] = useState(0);
  const canvas = useRef<HTMLCanvasElement>(null);
  const params = useMemo(() => (drag ? { ...slot.params, ...drag } : slot.params), [slot.params, drag]);

  /** Applies new values as one undoable edit and records local history. */
  const apply = (next: Record<string, number>, record = true) => {
    if (record) setHistory((h) => ({ undo: [...h.undo.slice(-99), slot.params], redo: [] }));
    commit({ ...slot, params: next });
  };
  const set = (k: string, v: number) => apply({ ...slot.params, [k]: v });

  useEffect(() => {
    let alive = true;
    effectsIpc
      .eqResponse(params, 300)
      .then((r) => alive && setCurve(r))
      .catch(() => alive && setCurve(null));
    return () => {
      alive = false;
    };
  }, [params]);

  const presetIndex = presets.findIndex((p) => Object.entries(p.params).every(([k, v]) => Math.abs((slot.params[k] ?? v) - v) < 1e-9));
  const loadPreset = (i: number) => {
    const p = presets[(i + presets.length) % presets.length];
    if (p) apply({ ...p.params });
  };

  // ---- drawing ----
  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    el.width = Math.max(1, Math.round(rect.width * dpr));
    el.height = Math.max(1, Math.round(rect.height * dpr));
    const ctx = el.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    const w = rect.width;
    const h = rect.height;
    const right = w - RIGHT;
    const x = (hz: number) => LEFT + (Math.log(hz / 20) / Math.log(1000)) * (right - LEFT);
    const mid = TOP + (h - TOP - BOTTOM) / 2;
    const y = (db: number) => mid - (db / scale) * ((h - TOP - BOTTOM) / 2);
    const bg = ctx.createLinearGradient(0, 0, 0, h);
    bg.addColorStop(0, "#0d1b2b");
    bg.addColorStop(1, "#0a1420");
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, w, h);
    ctx.font = "10px system-ui, sans-serif";
    // Grid: decades and their steps, dB lines.
    for (let d = 10; d <= 10000; d *= 10) {
      for (let k = 1; k < 10; k++) {
        const f = d * k;
        if (f < 20 || f > 20000) continue;
        ctx.strokeStyle = k === 1 ? "#1f3348" : "#15263a";
        ctx.beginPath();
        ctx.moveTo(x(f), TOP);
        ctx.lineTo(x(f), h - BOTTOM);
        ctx.stroke();
      }
    }
    const step = scale <= 6 ? 2 : scale <= 15 ? 5 : 6;
    for (let db = -scale; db <= scale; db += step) {
      ctx.strokeStyle = db === 0 ? "#3b5873" : "#15263a";
      ctx.beginPath();
      ctx.moveTo(LEFT, y(db));
      ctx.lineTo(right, y(db));
      ctx.stroke();
      ctx.fillStyle = "#6f8aa3";
      ctx.textAlign = "left";
      ctx.fillText(`${db > 0 ? "+" : ""}${db}`, right + 6, y(db) + 3);
    }
    // Analyser scale (dBFS) on the left.
    ctx.textAlign = "right";
    for (let a = 0; a <= 60; a += 10) {
      ctx.fillStyle = "#4f6b84";
      ctx.fillText(String(-a), LEFT - 6, TOP + (a / 60) * (h - TOP - BOTTOM) + 3);
    }
    // Frequency labels along the 0 dB line.
    ctx.textAlign = "center";
    ctx.fillStyle = "#8ea6bd";
    for (const f of [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]) {
      ctx.fillText(f >= 1000 ? `${f / 1000}k` : String(f), x(f), mid + 13);
    }
    // Measured spectrum: 0 dBFS at the top, −60 dBFS at the bottom.
    if (analyzer && spectrum) {
      ctx.beginPath();
      ctx.moveTo(x(spectrum.freqs[0] ?? 20), h - BOTTOM);
      spectrum.freqs.forEach((f, i) => {
        const level = Math.max(-60, Math.min(0, spectrum.db[i] ?? -140));
        ctx.lineTo(x(f), TOP + (-level / 60) * (h - TOP - BOTTOM));
      });
      ctx.lineTo(x(spectrum.freqs[spectrum.freqs.length - 1] ?? 20000), h - BOTTOM);
      ctx.closePath();
      ctx.fillStyle = "rgba(120, 170, 220, 0.22)";
      ctx.fill();
    }
    const clampY = (db: number) => y(Math.max(-scale * 1.2, Math.min(scale * 1.2, db)));
    // Selected band column.
    const selFreq = params[key(selected, "Freq")] ?? 1000;
    if ((params[key(selected, "On")] ?? 0) >= 0.5) {
      ctx.fillStyle = BAND_COLORS[selected]!;
      ctx.globalAlpha = 0.12;
      ctx.fillRect(x(selFreq) - 32, TOP, 64, h - TOP - BOTTOM);
      ctx.globalAlpha = 1;
    }
    if (curve) {
      // Each band's own area from the 0 dB line.
      for (const b of curve.bands) {
        ctx.beginPath();
        ctx.moveTo(x(curve.freqs[0]!), y(0));
        curve.freqs.forEach((f, i) => ctx.lineTo(x(f), clampY(b.db[i] ?? 0)));
        ctx.lineTo(x(curve.freqs[curve.freqs.length - 1]!), y(0));
        ctx.closePath();
        ctx.fillStyle = BAND_COLORS[b.band]!;
        ctx.globalAlpha = b.band === selected ? 0.32 : 0.16;
        ctx.fill();
        ctx.globalAlpha = 1;
      }
      // The whole equaliser.
      ctx.beginPath();
      curve.freqs.forEach((f, i) => (i === 0 ? ctx.moveTo(x(f), clampY(curve.db[i] ?? 0)) : ctx.lineTo(x(f), clampY(curve.db[i] ?? 0))));
      ctx.strokeStyle = bypass ? "#5d7084" : "#dfeaf3";
      ctx.lineWidth = 2;
      ctx.stroke();
      ctx.lineWidth = 1;
    }
    // Band handles.
    bands.forEach((_, b) => {
      const on = (params[key(b, "On")] ?? 0) >= 0.5;
      const type = Math.round(params[key(b, "Type")] ?? 2);
      const f = params[key(b, "Freq")] ?? 1000;
      // Handles sit on their band's own curve (bells at their gain, shelves at
      // half of it, cuts at −3 dB), where the band acts at its frequency.
      const own = curve?.bands.find((x) => x.band === b);
      let g = type === 2 ? (params[key(b, "Gain")] ?? 0) : 0;
      if (own && curve && type !== 2) {
        const hi = curve.freqs[curve.freqs.length - 1] ?? 20000;
        const i = Math.round((Math.log(f / 20) / Math.log(hi / 20)) * (curve.freqs.length - 1));
        g = own.db[Math.max(0, Math.min(own.db.length - 1, i))] ?? 0;
      }
      ctx.beginPath();
      ctx.arc(x(f), clampY(g), b === selected ? 6.5 : 5, 0, Math.PI * 2);
      ctx.fillStyle = BAND_COLORS[b]!;
      ctx.globalAlpha = on ? 1 : 0.28;
      ctx.fill();
      ctx.globalAlpha = 1;
    });
    // Output gain handle on the right edge.
    const out = params.outputDb ?? 0;
    ctx.beginPath();
    ctx.arc(right + 44, clampY(out), 7, 0, Math.PI * 2);
    ctx.strokeStyle = "#7fb7e6";
    ctx.lineWidth = 1.5;
    ctx.stroke();
    ctx.lineWidth = 1;
  }, [curve, spectrum, analyzer, params, bands, selected, scale, bypass]);

  // ---- graph interaction ----
  const geometry = (e: ReactPointerEvent<HTMLCanvasElement> | WheelEvent) => {
    const r = (e.currentTarget as HTMLCanvasElement).getBoundingClientRect();
    const right = r.width - RIGHT;
    const px = e.clientX - r.left;
    const py = e.clientY - r.top;
    const freq = 20 * Math.pow(1000, Math.min(1, Math.max(0, (px - LEFT) / (right - LEFT))));
    const mid = TOP + (r.height - TOP - BOTTOM) / 2;
    const gain = ((mid - py) / ((r.height - TOP - BOTTOM) / 2)) * scale;
    return { px, right, freq, gain };
  };
  const dragging = useRef<{ band: number | "out"; startY: number; startQ: number } | null>(null);
  const nearest = (freq: number) => {
    let best = selected;
    let dist = Infinity;
    bands.forEach((_, b) => {
      const d = Math.abs(Math.log(freq / (slot.params[key(b, "Freq")] ?? 1000)));
      if (d < dist) {
        dist = d;
        best = b;
      }
    });
    return best;
  };
  const freqParam = bands[0]!.find((p) => p.key.endsWith("Freq"))!;
  const gainParam = bands[0]!.find((p) => p.key.endsWith("Gain"))!;
  const qParam = bands[0]!.find((p) => p.key.endsWith("Q"))!;

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    // Wheel over the graph changes the selected band's Q (non-passive listener).
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const k = key(selected, "Q");
      const q = slot.params[k] ?? 1;
      set(k, fromPosition(qParam, toPosition(qParam, q) + (e.deltaY < 0 ? 0.02 : -0.02)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  });

  const toggleBand = (b: number) => {
    setSelected(b);
    set(key(b, "On"), (slot.params[key(b, "On")] ?? 0) >= 0.5 ? 0 : 1);
  };

  const undo = () => {
    const prev = history.undo[history.undo.length - 1];
    if (!prev) return;
    setHistory({ undo: history.undo.slice(0, -1), redo: [...history.redo, slot.params] });
    apply(prev, false);
  };
  const redo = () => {
    const next = history.redo[history.redo.length - 1];
    if (!next) return;
    setHistory({ undo: [...history.undo, slot.params], redo: history.redo.slice(0, -1) });
    apply(next, false);
  };

  return (
    <div className={`peq${bypass ? " is-bypassed" : ""}`}>
      <div className="peq__top">
        <button type="button" className={`peq__power${bypass ? "" : " is-on"}`} aria-pressed={!bypass} aria-label={bypass ? "Activar el ecualizador" : "Desactivar el ecualizador (bypass)"} title="Bypass sin clics" onClick={() => commit(slot, !bypass)}>
          <svg width="20" height="20" viewBox="0 0 20 20" aria-hidden>
            <path d="M10 2.5v7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
            <path d="M5.6 5.2a6.5 6.5 0 1 0 8.8 0" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
          </svg>
        </button>
        <div className="peq__presetbox">
          <select
            className="peq__preset"
            aria-label="Preset"
            value={presetIndex >= 0 ? String(presetIndex) : saveName !== null ? "save" : "custom"}
            onChange={(e) => {
              if (e.target.value === "save") setSaveName("");
              else if (e.target.value !== "custom") loadPreset(Number(e.target.value));
            }}
          >
            {presetIndex < 0 && <option value="custom">Ajuste propio</option>}
            <optgroup label="Elyvon (originales)">
              {presets.map((p, i) => (p.source === "factory" ? <option key={`f${i}`} value={i}>{p.name}</option> : null))}
            </optgroup>
            {presets.some((p) => p.source === "user") && (
              <optgroup label="Mis presets">
                {presets.map((p, i) => (p.source === "user" ? <option key={`u${i}`} value={i}>{p.name}</option> : null))}
              </optgroup>
            )}
            <option value="save">Guardar el ajuste como preset…</option>
          </select>
          <div className="peq__commands">
            <button type="button" aria-label="Preset anterior" onClick={() => loadPreset((presetIndex < 0 ? 0 : presetIndex) - 1)}>‹</button>
            <button type="button" aria-label="Preset siguiente" onClick={() => loadPreset(presetIndex + 1)}>›</button>
            <button type="button" className={slot.alternate ? "is-on" : ""} title="Comparar: alterna entre este ajuste y el alternativo (la primera vez guarda una copia)" onClick={() => commit({ ...slot, params: slot.alternate ?? slot.params, alternate: slot.params })}>
              Comparar
            </button>
            <button
              type="button"
              title="Copia el ajuste del ecualizador"
              onClick={() => {
                clipboard = { ...slot.params };
                bump((x) => x + 1);
              }}
            >
              Copiar
            </button>
            <button type="button" disabled={!clipboard} title="Pega el ajuste copiado" onClick={() => clipboard && apply({ ...clipboard })}>
              Pegar
            </button>
            <button type="button" disabled={!history.undo.length} title="Deshace el último cambio de este ecualizador" onClick={undo}>
              Deshacer
            </button>
            <button type="button" disabled={!history.redo.length} title="Rehace el cambio deshecho" onClick={redo}>
              Rehacer
            </button>
          </div>
        </div>
        <span className="peq__spacer" />
        <label className="peq__scale">
          Escala:
          <select aria-label="Escala de ganancia" value={scale} onChange={(e) => setScale(Number(e.target.value))}>
            {SCALES.map((s) => (
              <option key={s} value={s}>
                ±{s} dB
              </option>
            ))}
          </select>
        </label>
      </div>
      {saveName !== null && (
        <div className="peq__save">
          <input autoFocus aria-label="Nombre del preset" placeholder="Nombre del preset" maxLength={96} value={saveName} onChange={(e) => setSaveName(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
          <button
            type="button"
            disabled={!saveName.trim()}
            onClick={() =>
              effectsIpc
                .savePreset(info.kind, saveName, slot.params)
                .then(() => effectsIpc.presets(info.kind))
                .then((list) => {
                  setPresets(list);
                  notify("success", `Preset guardado: ${saveName.trim()}`);
                  setSaveName(null);
                }, notifyError)
            }
          >
            Guardar
          </button>
          <button
            type="button"
            disabled={!presets.some((p) => p.source === "user" && p.name === saveName.trim())}
            onClick={() =>
              effectsIpc
                .deletePreset(info.kind, saveName.trim())
                .then(() => effectsIpc.presets(info.kind))
                .then((list) => {
                  setPresets(list);
                  setSaveName(null);
                }, notifyError)
            }
          >
            Borrar
          </button>
          <button type="button" onClick={() => setSaveName(null)}>
            Cancelar
          </button>
        </div>
      )}
      <div className="peq__display">
        <div className="peq__glyphs" style={{ paddingLeft: LEFT, paddingRight: RIGHT }}>
          {bands.map((_, b) => {
            const on = (slot.params[key(b, "On")] ?? 0) >= 0.5;
            const type = Math.round(slot.params[key(b, "Type")] ?? 2);
            return (
              <button key={b} type="button" className={`peq__glyph${b === selected ? " is-selected" : ""}`} aria-pressed={on} aria-label={`Banda ${b + 1} ${on ? "activa" : "desactivada"}`} title="Clic: activar o desactivar la banda" onClick={() => toggleBand(b)}>
                <Glyph type={type} color={on ? BAND_COLORS[b]! : "#5f778c"} />
              </button>
            );
          })}
        </div>
        <canvas
          ref={canvas}
          className="peq__graph"
          role="img"
          aria-label="Respuesta del ecualizador y espectro medido. Arrastra una banda para mover su frecuencia y ganancia; la rueda cambia su Q"
          onPointerDown={(e) => {
            const g = geometry(e);
            e.currentTarget.setPointerCapture(e.pointerId);
            if (g.px > g.right + 30) {
              dragging.current = { band: "out", startY: e.clientY, startQ: 0 };
              setDrag({ outputDb: clampParam(output, Math.round(g.gain * 10) / 10) });
              return;
            }
            const b = nearest(g.freq);
            setSelected(b);
            dragging.current = { band: b, startY: e.clientY, startQ: slot.params[key(b, "Q")] ?? 1 };
          }}
          onPointerMove={(e) => {
            const d = dragging.current;
            if (!d) return;
            const g = geometry(e);
            if (d.band === "out") {
              setDrag({ outputDb: clampParam(output, Math.round(g.gain * 10) / 10) });
              return;
            }
            const b = d.band;
            const type = Math.round(slot.params[key(b, "Type")] ?? 2);
            const next: Record<string, number> = { [key(b, "Freq")]: clampParam(freqParam, Math.round(g.freq)), [key(b, "On")]: 1 };
            // Shelves are grabbed at half their gain (where their handle sits).
            if (bandParamVisible(type, "b1Gain")) next[key(b, "Gain")] = clampParam(gainParam, Math.round(g.gain * (type === 2 ? 10 : 20)) / 10);
            else {
              // Cuts and band shapes: vertical movement changes Q.
              const pos = toPosition(qParam, d.startQ) + (d.startY - e.clientY) / 300;
              next[key(b, "Q")] = fromPosition(qParam, pos);
            }
            setDrag(next);
          }}
          onPointerUp={() => {
            if (drag) apply({ ...slot.params, ...drag });
            dragging.current = null;
            setDrag(null);
          }}
          onPointerCancel={() => {
            dragging.current = null;
            setDrag(null);
          }}
        />
        <div className="peq__values" style={{ paddingLeft: LEFT }}>
          {bands.map((band, b) => {
            const on = (params[key(b, "On")] ?? 0) >= 0.5;
            const type = Math.round(params[key(b, "Type")] ?? 2);
            const p = (name: string) => band.find((x) => x.key === key(b, name))!;
            const second = bandParamVisible(type, key(b, "Gain")) ? p("Gain") : bandParamVisible(type, key(b, "Slope")) ? p("Slope") : null;
            const color = BAND_COLORS[b]!;
            return (
              <div key={b} className={`peq__band${b === selected ? " is-selected" : ""}`} onPointerDown={() => setSelected(b)} style={b === selected ? { background: `${color}26` } : undefined}>
                <DragValue p={p("Freq")} value={paramValue({ ...slot, params }, p("Freq"))} onCommit={(v) => set(key(b, "Freq"), v)} color={color} title={`Banda ${b + 1}: frecuencia`} dim={!on} />
                {second ? (
                  <DragValue p={second} value={paramValue({ ...slot, params }, second)} onCommit={(v) => set(second.key, v)} color={color} title={`Banda ${b + 1}: ${second.key.endsWith("Gain") ? "ganancia" : "pendiente"}`} dim={!on} />
                ) : (
                  <span className="peq-value is-dim">—</span>
                )}
                <DragValue p={p("Q")} value={paramValue({ ...slot, params }, p("Q"))} onCommit={(v) => set(key(b, "Q"), v)} color={color} title={`Banda ${b + 1}: Q`} dim={!on} />
                <select className="peq__type" aria-label={`Banda ${b + 1}: tipo`} value={type} onChange={(e) => set(key(b, "Type"), Number(e.target.value))} style={{ color: on ? color : undefined }}>
                  {p("Type").choices.map((c, i) => (
                    <option key={c} value={i}>
                      {c}
                    </option>
                  ))}
                </select>
              </div>
            );
          })}
          <div className="peq__band peq__band--gain">
            <span className="peq__gain-label">Ganancia</span>
            <DragValue p={output} value={params.outputDb ?? 0} onCommit={(v) => set("outputDb", v)} color="#cfe3f5" title="Ganancia de salida" />
          </div>
        </div>
        <div className="peq__bottom">
          <button type="button" className={analyzer ? "is-on" : ""} aria-pressed={analyzer} title="Espectro de la salida del canal (post fader), calculado fuera del hilo de audio" onClick={() => setAnalyzer(!analyzer)}>
            Analizador <sup>POST</sup>
          </button>
          <button type="button" disabled title="Acoplamiento automático de Q: no disponible todavía">
            Acoplar Q
          </button>
          <button type="button" disabled title="Procesado de alta resolución: no disponible todavía (el ecualizador ya calcula en doble precisión)">
            HQ
          </button>
          {processing && (
            <label>
              Procesado:
              <select aria-label="Procesado" value={Math.round(params.processing ?? 0)} onChange={(e) => set("processing", Number(e.target.value))}>
                {processing.choices.map((c, i) => (
                  <option key={c} value={i}>
                    {c}
                  </option>
                ))}
              </select>
            </label>
          )}
          <span className="peq__spacer" />
          <span className="peq__latency">{latency}</span>
        </div>
      </div>
      <div className="peq__name">Ecualizador paramétrico</div>
    </div>
  );
}
