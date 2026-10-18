import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import type { ChannelRef } from "../../bindings/ChannelRef";
import type { EffectDetails } from "../../bindings/EffectDetails";
import type { EffectInfo } from "../../bindings/EffectInfo";
import type { EffectParamInfo } from "../../bindings/EffectParamInfo";
import type { EffectSlot } from "../../bindings/EffectSlot";
import type { InsertSlot } from "../../bindings/InsertSlot";
import type { PresetEntry } from "../../bindings/PresetEntry";
import type { Project } from "../../bindings/Project";
import type { Spectrum } from "../../bindings/Spectrum";
import { Dialog } from "../../components/Dialog";
import {
  bandParamVisible,
  clampParam,
  effectInfo,
  effectsIpc,
  eqBands,
  formatParam,
  fromPosition,
  loadCatalog,
  paramValue,
  plainParams,
  toPosition,
} from "../../lib/effects";
import { notify, notifyError } from "../../state/notifications";
import { useProject } from "../../state/project";
import { useTransport } from "../../state/transport";
import "./effects.css";

/** The insert slots of a channel strip. */
function insertsOf(project: Project, c: ChannelRef): InsertSlot[] {
  if (c.type === "track") return project.tracks.find((t) => t.id === c.id)?.strip.inserts ?? [];
  if (c.type === "aux") return project.mixer.auxes.find((a) => a.id === c.id)?.strip.inserts ?? [];
  return project.mixer.master.inserts;
}

function channelName(project: Project, c: ChannelRef): string {
  if (c.type === "track") return project.tracks.find((t) => t.id === c.id)?.name ?? "Pista";
  if (c.type === "aux") return project.mixer.auxes.find((a) => a.id === c.id)?.name ?? "Auxiliar";
  return "Stereo Out";
}

/** Gain reduction (dB) of insert `index` of `c` in the latest meter reading. */
function useReduction(project: Project | null, c: ChannelRef, index: number): number | null {
  const strip = useMemo(() => {
    if (!project || c.type === "master") return { type: "master" } as const;
    if (c.type === "track") return { type: "track", index: project.tracks.findIndex((t) => t.id === c.id) } as const;
    return { type: "aux", index: project.mixer.auxes.findIndex((a) => a.id === c.id) } as const;
  }, [project, c]);
  const [peak, setPeak] = useState<number | null>(null);
  useEffect(() => {
    let decay = 0;
    return useTransport.subscribe((s) => {
      const r = s.meters?.reduction.find(
        (x) => x.index === index && x.strip.type === strip.type && (strip.type === "master" || ("index" in x.strip && x.strip.index === strip.index)),
      );
      // Peak hold with a ~1 s fall so short reductions stay readable.
      decay = Math.max(r?.reductionDb ?? 0, decay * 0.85);
      setPeak(decay);
    });
  }, [strip, index]);
  return peak;
}

/** One parameter: slider (continuous), menu (choice) or switch (toggle). */
function ParamControl({ p, value, onCommit, disabled }: { p: EffectParamInfo; value: number; onCommit: (v: number) => void; disabled?: boolean }) {
  const [draft, setDraft] = useState<number | null>(null);
  const shown = draft ?? value;
  const id = `fx-${p.key}`;
  if (p.scale === "toggle") {
    return (
      <label className="fx-param fx-param--toggle">
        <input type="checkbox" checked={value >= 0.5} disabled={disabled} onChange={(e) => onCommit(e.target.checked ? 1 : 0)} />
        <span>{p.label}</span>
      </label>
    );
  }
  if (p.scale === "choice") {
    return (
      <label className="fx-param">
        <span className="fx-param__label">{p.label}</span>
        <select aria-label={p.label} value={Math.round(value)} disabled={disabled} onChange={(e) => onCommit(Number(e.target.value))}>
          {p.choices.map((c, i) => (
            <option key={c} value={i}>
              {c}
            </option>
          ))}
        </select>
      </label>
    );
  }
  const commit = () => {
    if (draft !== null && draft !== value) onCommit(draft);
    setDraft(null);
  };
  return (
    <div className="fx-param">
      <label className="fx-param__label" htmlFor={id}>
        {p.label}
        {!p.automatable && <span className="fx-param__fixed" title="Cambiarlo reconstruye el efecto; no es automatizable"> · fijo</span>}
      </label>
      <input
        id={id}
        type="range"
        min={0}
        max={1000}
        disabled={disabled}
        value={Math.round(toPosition(p, shown) * 1000)}
        aria-valuetext={formatParam(p, shown)}
        onChange={(e) => setDraft(fromPosition(p, Number(e.target.value) / 1000))}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
        onDoubleClick={() => onCommit(p.default)}
        title="Doble clic: valor por omisión"
      />
      <ValueField p={p} value={shown} onCommit={onCommit} disabled={disabled} />
    </div>
  );
}

/** Typed value entry. */
function ValueField({ p, value, onCommit, disabled }: { p: EffectParamInfo; value: number; onCommit: (v: number) => void; disabled?: boolean }) {
  const [text, setText] = useState<string | null>(null);
  if (text === null) {
    return (
      <button type="button" className="fx-param__value" disabled={disabled} onClick={() => setText(String(Number(value.toFixed(3))))}>
        {formatParam(p, value)}
      </button>
    );
  }
  const done = (save: boolean) => {
    const v = Number(text.replace(",", "."));
    setText(null);
    if (save && Number.isFinite(v)) onCommit(clampParam(p, v));
  };
  return (
    <input
      className="fx-param__input"
      autoFocus
      aria-label={`${p.label} (valor)`}
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

const BAND_COLORS = ["#8aa7c0", "#e09a2c", "#d6d23a", "#59d65a", "#2fd3b0", "#63bdea", "#b98af0", "#8aa7c0"];

/**
 * Equaliser display: the exact response of the current setting computed in
 * Rust (the same biquads the engine runs) over the channel's measured
 * spectrum (analysed off the audio thread). Dragging a band's handle moves
 * its frequency and gain.
 */
function EqGraph({ slot, info, spectrum, onBand }: { slot: EffectSlot; info: EffectInfo; spectrum: Spectrum | null; onBand: (band: number, freq: number, gain: number) => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [curve, setCurve] = useState<{ freqs: number[]; db: number[] } | null>(null);
  const [drag, setDrag] = useState<{ band: number; freq: number; gain: number } | null>(null);
  const bands = useMemo(() => eqBands(info), [info]);
  // The drawn parameters include a handle being dragged.
  const params = useMemo(() => {
    if (!drag) return slot.params;
    return { ...slot.params, [`b${drag.band + 1}Freq`]: drag.freq, [`b${drag.band + 1}Gain`]: drag.gain };
  }, [slot.params, drag]);
  useEffect(() => {
    let alive = true;
    effectsIpc
      .eqResponse(params, 256)
      .then((r) => alive && setCurve({ freqs: r.freqs, db: r.db }))
      .catch(() => alive && setCurve(null));
    return () => {
      alive = false;
    };
  }, [params]);
  const range = 24;
  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    el.width = Math.max(1, rect.width * dpr);
    el.height = Math.max(1, rect.height * dpr);
    const ctx = el.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    const w = rect.width;
    const h = rect.height;
    const x = (hz: number) => (Math.log(hz / 20) / Math.log(1000)) * w;
    const y = (db: number) => h / 2 - (db / range) * (h / 2 - 8);
    ctx.fillStyle = "#11171d";
    ctx.fillRect(0, 0, w, h);
    ctx.lineWidth = 1;
    ctx.font = "10px system-ui, sans-serif";
    for (const hz of [50, 100, 200, 500, 1000, 2000, 5000, 10000]) {
      ctx.strokeStyle = "#1f2a35";
      ctx.beginPath();
      ctx.moveTo(x(hz), 0);
      ctx.lineTo(x(hz), h);
      ctx.stroke();
      ctx.fillStyle = "#6f8496";
      ctx.fillText(hz >= 1000 ? `${hz / 1000}k` : String(hz), x(hz) + 2, h - 4);
    }
    for (const db of [-18, -12, -6, 0, 6, 12, 18]) {
      ctx.strokeStyle = db === 0 ? "#3d5468" : "#1f2a35";
      ctx.beginPath();
      ctx.moveTo(0, y(db));
      ctx.lineTo(w, y(db));
      ctx.stroke();
      ctx.fillStyle = "#6f8496";
      ctx.fillText(`${db > 0 ? "+" : ""}${db}`, 2, y(db) - 2);
    }
    // Measured spectrum (dBFS, −90 … 0 mapped to the bottom half).
    if (spectrum) {
      ctx.beginPath();
      spectrum.freqs.forEach((f, i) => {
        const level = Math.max(-90, spectrum.db[i] ?? -140);
        const py = h - ((level + 90) / 90) * (h * 0.9);
        if (i === 0) ctx.moveTo(x(f), py);
        else ctx.lineTo(x(f), py);
      });
      ctx.lineTo(w, h);
      ctx.lineTo(0, h);
      ctx.closePath();
      ctx.fillStyle = "rgba(99, 189, 234, 0.16)";
      ctx.fill();
    }
    if (curve) {
      ctx.beginPath();
      curve.freqs.forEach((f, i) => {
        const py = y(Math.max(-range * 1.5, Math.min(range * 1.5, curve.db[i] ?? 0)));
        if (i === 0) ctx.moveTo(x(f), py);
        else ctx.lineTo(x(f), py);
      });
      ctx.strokeStyle = "#e8eef3";
      ctx.lineWidth = 2;
      ctx.stroke();
    }
    bands.forEach((band, b) => {
      const on = (params[`b${b + 1}On`] ?? 0) >= 0.5;
      const type = Math.round(params[`b${b + 1}Type`] ?? 2);
      const freq = params[`b${b + 1}Freq`] ?? 1000;
      const gain = bandParamVisible(type, "b1Gain") ? (params[`b${b + 1}Gain`] ?? 0) : 0;
      ctx.beginPath();
      ctx.arc(x(freq), y(gain), 6, 0, Math.PI * 2);
      ctx.fillStyle = BAND_COLORS[b] ?? "#fff";
      ctx.globalAlpha = on ? 1 : 0.3;
      ctx.fill();
      ctx.globalAlpha = 1;
      ctx.fillStyle = "#11171d";
      ctx.fillText(String(b + 1), x(freq) - 3, y(gain) + 3.5);
      void band;
    });
  }, [curve, spectrum, bands, params]);
  const fromPointer = (e: PointerEvent<HTMLCanvasElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const freq = 20 * Math.pow(1000, Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)));
    const gain = ((r.height / 2 - (e.clientY - r.top)) / (r.height / 2 - 8)) * range;
    return { freq: Math.round(Math.min(20000, Math.max(20, freq))), gain: Math.round(Math.min(24, Math.max(-24, gain)) * 10) / 10 };
  };
  const nearest = (freq: number) => {
    let best = 0;
    let dist = Infinity;
    bands.forEach((_, b) => {
      const d = Math.abs(Math.log(freq / (slot.params[`b${b + 1}Freq`] ?? 1000)));
      if (d < dist) {
        dist = d;
        best = b;
      }
    });
    return best;
  };
  return (
    <canvas
      ref={canvas}
      className="fx-eq__graph"
      role="img"
      aria-label="Respuesta del ecualizador calculada por el motor y espectro medido; arrastra un punto para mover su banda"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        const p = fromPointer(e);
        setDrag({ band: nearest(p.freq), ...p });
      }}
      onPointerMove={(e) => {
        if (drag) setDrag({ band: drag.band, ...fromPointer(e) });
      }}
      onPointerUp={() => {
        if (drag) onBand(drag.band, drag.freq, drag.gain);
        setDrag(null);
      }}
      onPointerCancel={() => setDrag(null)}
    />
  );
}

/** Horizontal gain-reduction meter (0 … 40 dB). */
function ReductionMeter({ db }: { db: number | null }) {
  const v = Math.min(40, Math.max(0, db ?? 0));
  return (
    <div className="fx-gr" role="meter" aria-label="Reducción de ganancia" aria-valuemin={0} aria-valuemax={40} aria-valuenow={Number(v.toFixed(1))}>
      <span className="fx-gr__label">GR</span>
      <div className="fx-gr__track">
        <div className="fx-gr__bar" style={{ width: `${(v / 40) * 100}%` }} />
      </div>
      <span className="fx-gr__value">{db === null ? "—" : `−${v.toFixed(1).replace(".", ",")} dB`}</span>
    </div>
  );
}

/** Editor of one native effect insert. Every change is an undoable `setInsert`. */
export function EffectEditor({ channel, insertId, onClose }: { channel: ChannelRef; insertId: string; onClose: () => void }) {
  const project = useProject((s) => s.view?.project ?? null);
  const apply = useProject((s) => s.apply);
  const [info, setInfo] = useState<EffectInfo | undefined>(undefined);
  const [presets, setPresets] = useState<PresetEntry[]>([]);
  const [details, setDetails] = useState<EffectDetails | null>(null);
  const [spectrum, setSpectrum] = useState<Spectrum | null>(null);
  const [presetName, setPresetName] = useState("");
  const inserts = project ? insertsOf(project, channel) : [];
  const index = inserts.findIndex((s) => s.id === insertId);
  const slot = inserts[index];
  const effect = slot?.processor.type === "effect" ? (slot.processor as { type: "effect" } & EffectSlot) : null;
  const kind = effect?.kind;

  useEffect(() => {
    if (!kind) return;
    void loadCatalog().then(() => setInfo(effectInfo(kind)));
    effectsIpc.presets(kind).then(setPresets, notifyError);
  }, [kind]);

  useEffect(() => {
    if (!effect) return;
    let alive = true;
    const { type: _t, ...plain } = effect;
    void _t;
    effectsIpc.details(plain).then((d) => alive && setDetails(d), () => alive && setDetails(null));
    return () => {
      alive = false;
    };
  }, [effect]);

  // Spectrum of this channel, analysed off the audio thread.
  const channelKey = JSON.stringify(channel);
  useEffect(() => {
    if (kind !== "parametricEq") return;
    const c = JSON.parse(channelKey) as ChannelRef;
    void effectsIpc.analyzerSelect(c).catch(() => undefined);
    const timer = window.setInterval(() => {
      effectsIpc.analyzerSpectrum().then(setSpectrum, () => undefined);
    }, 100);
    return () => {
      window.clearInterval(timer);
      void effectsIpc.analyzerSelect(null).catch(() => undefined);
    };
  }, [kind, channelKey]);

  const reduction = useReduction(project, channel, Math.max(0, index));

  const commit = useCallback(
    (next: EffectSlot, bypass = slot?.bypass ?? false) => {
      if (!slot) return;
      void apply({ type: "setInsert", channel, id: slot.id, bypass, processor: { type: "effect", ...next } });
    },
    [apply, channel, slot],
  );

  useEffect(() => {
    if (project && !effect) onClose();
  }, [project, effect, onClose]);
  if (!project || !effect || !slot) return null;
  const { type: _type, ...current } = effect;
  void _type;
  const setParam = (key: string, v: number) => commit({ ...current, params: { ...current.params, [key]: v } });

  const title = `${info?.label ?? kind} — ${channelName(project, channel)} · inserto ${index + 1}`;
  const presetOptions = presets.map((p, i) => ({ id: `${p.source}:${i}`, p }));
  const tracks = project.tracks.filter((t) => !(channel.type === "track" && t.id === channel.id) && t.kind !== "midi");

  return (
    <Dialog open title={title} onClose={onClose} width={kind === "parametricEq" ? 900 : 640} className="fx-editor">
      <div className="fx-editor__bar">
        <button type="button" className={`fx-btn${slot.bypass ? "" : " is-on"}`} aria-pressed={!slot.bypass} onClick={() => commit(current, !slot.bypass)} title="Bypass sin clics (fundido de un bloque)">
          {slot.bypass ? "Desactivado" : "Activo"}
        </button>
        <span className="fx-editor__ab" role="group" aria-label="Comparación A/B">
          <button type="button" className="fx-btn" onClick={() => commit({ ...current, params: current.alternate ?? current.params, alternate: current.params })} title="Intercambia el ajuste actual con el alternativo">
            A ⇄ B
          </button>
          <button type="button" className="fx-btn" onClick={() => commit({ ...current, alternate: { ...current.params } })} title="Guarda el ajuste actual como alternativa">
            Copiar a la alternativa
          </button>
          <span className="fx-editor__hint">{current.alternate ? "Alternativa guardada" : "Sin alternativa"}</span>
        </span>
        <label className="fx-editor__preset">
          <span>Preset</span>
          <select
            aria-label="Cargar preset"
            value=""
            onChange={(e) => {
              const entry = presetOptions.find((o) => o.id === e.target.value)?.p;
              if (entry) commit({ ...current, params: { ...entry.params } });
            }}
          >
            <option value="" disabled>
              Cargar…
            </option>
            <optgroup label="Elyvon (originales)">
              {presetOptions.filter((o) => o.p.source === "factory").map((o) => (
                <option key={o.id} value={o.id}>
                  {o.p.name}
                </option>
              ))}
            </optgroup>
            {presetOptions.some((o) => o.p.source === "user") && (
              <optgroup label="Mis presets">
                {presetOptions.filter((o) => o.p.source === "user").map((o) => (
                  <option key={o.id} value={o.id}>
                    {o.p.name}
                  </option>
                ))}
              </optgroup>
            )}
          </select>
        </label>
        <span className="fx-editor__save">
          <input aria-label="Nombre del preset" placeholder="Nombre del preset" value={presetName} maxLength={96} onChange={(e) => setPresetName(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
          <button
            type="button"
            className="fx-btn"
            disabled={!presetName.trim() || !kind}
            onClick={() => {
              if (!kind) return;
              effectsIpc
                .savePreset(kind, presetName, current.params)
                .then(() => effectsIpc.presets(kind))
                .then((list) => {
                  setPresets(list);
                  notify("success", `Preset guardado: ${presetName.trim()}`);
                  setPresetName("");
                }, notifyError);
            }}
          >
            Guardar
          </button>
          <button
            type="button"
            className="fx-btn"
            disabled={!kind || !presets.some((p) => p.source === "user" && p.name === presetName.trim())}
            title="Borra el preset de usuario con ese nombre"
            onClick={() => {
              if (!kind) return;
              effectsIpc
                .deletePreset(kind, presetName.trim())
                .then(() => effectsIpc.presets(kind))
                .then((list) => {
                  setPresets(list);
                  setPresetName("");
                }, notifyError);
            }}
          >
            Borrar
          </button>
        </span>
      </div>
      <div className="fx-editor__status">
        <span>Latencia: {details ? `${details.latencyFrames} muestras (${details.latencyMs.toFixed(1).replace(".", ",")} ms, compensada)` : "—"}</span>
        <span>Cola: {details ? (details.tailSeconds === null ? "sin fin (congelada o realimentación total)" : `${details.tailSeconds.toFixed(1).replace(".", ",")} s`) : "—"}</span>
        {details && <span>{details.sampleRate} Hz</span>}
      </div>
      {info?.hasSidechain && (
        <div className="fx-editor__sidechain">
          <label>
            <span>Sidechain</span>
            <select
              aria-label="Fuente de sidechain"
              value={current.sidechain ?? ""}
              onChange={(e) => {
                const next: EffectSlot = { ...current, sidechain: e.target.value || undefined };
                if (e.target.value) next.params = { ...next.params, external: 1 };
                commit(next);
              }}
            >
              <option value="">Ninguna (detecta su propia señal)</option>
              {tracks.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </select>
          </label>
          <ReductionMeter db={reduction} />
        </div>
      )}
      {!info && <p className="fx-editor__empty">Cargando parámetros del motor…</p>}
      {info && kind === "parametricEq" && (
        <EqEditor info={info} slot={current} spectrum={spectrum} setParam={setParam} commit={commit} />
      )}
      {info && kind !== "parametricEq" && (
        <div className="fx-grid">
          {info.params.map((p) => (
            <ParamControl key={p.key} p={p} value={paramValue(current, p)} onCommit={(v) => setParam(p.key, v)} />
          ))}
        </div>
      )}
      <p className="fx-editor__note">
        Procesado nativo en Rust. Los cambios son ediciones con deshacer; los parámetros automatizables aparecen en las curvas de automatización del canal.
      </p>
    </Dialog>
  );
}

function EqEditor({ info, slot, spectrum, setParam, commit }: { info: EffectInfo; slot: EffectSlot; spectrum: Spectrum | null; setParam: (key: string, v: number) => void; commit: (s: EffectSlot) => void }) {
  const bands = eqBands(info);
  return (
    <div className="fx-eq">
      <EqGraph
        slot={slot}
        info={info}
        spectrum={spectrum}
        onBand={(b, freq, gain) => {
          const type = Math.round(slot.params[`b${b + 1}Type`] ?? 2);
          const params = { ...slot.params, [`b${b + 1}Freq`]: freq, [`b${b + 1}On`]: 1 };
          if (bandParamVisible(type, "b1Gain")) params[`b${b + 1}Gain`] = gain;
          commit({ ...slot, params });
        }}
      />
      <div className="fx-eq__bands">
        {bands.map((band, b) => {
          const type = Math.round(paramValue(slot, band[1]!));
          return (
            <fieldset key={b} className="fx-eq__band" style={{ borderTopColor: BAND_COLORS[b] }}>
              <legend>Banda {b + 1}</legend>
              {band
                .filter((p) => bandParamVisible(type, p.key))
                .map((p) => (
                  <ParamControl key={p.key} p={{ ...p, label: p.label.replace(/^Banda \d+ /, "") }} value={paramValue(slot, p)} onCommit={(v) => setParam(p.key, v)} />
                ))}
            </fieldset>
          );
        })}
      </div>
      <div className="fx-grid">
        {plainParams(info).map((p) => (
          <ParamControl key={p.key} p={p} value={paramValue(slot, p)} onCommit={(v) => setParam(p.key, v)} />
        ))}
      </div>
    </div>
  );
}
