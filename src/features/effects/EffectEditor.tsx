import { useCallback, useEffect, useMemo, useState } from "react";
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
import { clampParam, effectInfo, effectsIpc, formatParam, fromPosition, loadCatalog, paramValue, toPosition } from "../../lib/effects";
import { EqWindow } from "./EqWindow";
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
  const [analyzer, setAnalyzer] = useState(true);
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
    if (kind !== "parametricEq" || !analyzer) return;
    const c = JSON.parse(channelKey) as ChannelRef;
    void effectsIpc.analyzerSelect(c).catch(() => undefined);
    const timer = window.setInterval(() => {
      effectsIpc.analyzerSpectrum().then(setSpectrum, () => undefined);
    }, 100);
    return () => {
      window.clearInterval(timer);
      void effectsIpc.analyzerSelect(null).catch(() => undefined);
    };
  }, [kind, channelKey, analyzer]);

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
  const latency = details ? `Latencia: ${details.latencyFrames} muestras · ${details.sampleRate} Hz` : "";
  if (kind === "parametricEq" && info) {
    return (
      <Dialog open title={`${channelName(project, channel)} · inserto ${index + 1}`} onClose={onClose} width={1060} className="fx-editor peq-window">
        <EqWindow info={info} slot={current} bypass={slot.bypass} commit={commit} presets={presets} setPresets={setPresets} spectrum={analyzer ? spectrum : null} analyzer={analyzer} setAnalyzer={setAnalyzer} latency={latency} />
      </Dialog>
    );
  }
  const tracks = project.tracks.filter((t) => !(channel.type === "track" && t.id === channel.id) && t.kind !== "midi");

  return (
    <Dialog open title={title} onClose={onClose} width={640} className="fx-editor">
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
      {info && (
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

