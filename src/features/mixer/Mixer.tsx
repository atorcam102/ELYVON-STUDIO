import { memo, useEffect, useRef, useState, type PointerEvent } from "react";
import type { Track } from "../../bindings/Track";
import type { TrackKind } from "../../bindings/TrackKind";
import { Badge, Toggle } from "../../components/Button";
import { Icon } from "../../components/Icon";
import {
  dbToFaderPos,
  faderPosToDb,
  formatDb,
  formatPan,
  MIN_DB,
} from "../../lib/format";
import { engineRunning, useAudio } from "../../state/audio";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";

/** Registry of meter bar elements, animated without React re-renders. */
type MeterRegistry = Map<string, HTMLDivElement>;

function Fader({
  value,
  onCommit,
  label,
  disabled,
}: {
  value: number;
  onCommit: (db: number) => void;
  label: string;
  disabled?: boolean;
}) {
  const [preview, setPreview] = useState<number | null>(null);
  const drag = useRef<{ y: number; pos: number; height: number } | null>(null);
  const db = preview ?? value;
  const pos = dbToFaderPos(db);

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled || e.button !== 0) return;
    if (e.altKey) return onCommit(0);
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = {
      y: e.clientY,
      pos,
      height: Math.max(40, e.currentTarget.getBoundingClientRect().height),
    };
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const sensitivity = e.shiftKey ? 0.2 : 1;
    const p = Math.min(
      1,
      Math.max(
        0,
        drag.current.pos +
          ((drag.current.y - e.clientY) / drag.current.height) * sensitivity,
      ),
    );
    setPreview(Math.round(faderPosToDb(p) * 10) / 10);
  };
  const onUp = () => {
    if (drag.current && preview !== null && preview !== value)
      onCommit(Math.max(MIN_DB, preview));
    drag.current = null;
    setPreview(null);
  };

  return (
    <div
      className={`fader${disabled ? " is-disabled" : ""}`}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={MIN_DB}
      aria-valuemax={6}
      aria-valuenow={db}
      aria-valuetext={`${formatDb(db)} dB`}
      title={`${label}: ${formatDb(db)} dB — drag (Shift for fine), Alt-click or double-click for 0 dB`}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
      onDoubleClick={() => !disabled && onCommit(0)}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 0.1 : 1;
        if (e.key === "ArrowUp")
          onCommit(Math.min(6, Math.round((db + step) * 10) / 10));
        else if (e.key === "ArrowDown")
          onCommit(Math.max(MIN_DB, Math.round((db - step) * 10) / 10));
        else return;
        e.preventDefault();
      }}
    >
      <div className="fader__track" />
      <div
        className="fader__unity"
        style={{ bottom: `${dbToFaderPos(0) * 100}%` }}
      />
      <div
        className="fader__cap"
        style={{ bottom: `calc(${pos * 100}% - 14px)` }}
      />
    </div>
  );
}

function PanKnob({
  value,
  onCommit,
  disabled,
}: {
  value: number;
  onCommit: (pan: number) => void;
  disabled?: boolean;
}) {
  const [preview, setPreview] = useState<number | null>(null);
  const drag = useRef<{ y: number; v: number } | null>(null);
  const v = preview ?? value;
  const angle = v * 135;
  return (
    <div
      className={`knob${disabled ? " is-disabled" : ""}`}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label="Pan"
      aria-valuemin={-1}
      aria-valuemax={1}
      aria-valuenow={v}
      aria-valuetext={formatPan(v)}
      title={`Pan ${formatPan(v)} — drag vertically, double-click to centre`}
      onPointerDown={(e) => {
        if (disabled) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { y: e.clientY, v: value };
      }}
      onPointerMove={(e) => {
        if (!drag.current) return;
        const next = Math.min(
          1,
          Math.max(-1, drag.current.v + (drag.current.y - e.clientY) / 100),
        );
        setPreview(Math.round(next * 64) / 64);
      }}
      onPointerUp={() => {
        if (drag.current && preview !== null && preview !== value)
          onCommit(preview);
        drag.current = null;
        setPreview(null);
      }}
      onDoubleClick={() => !disabled && onCommit(0)}
      onKeyDown={(e) => {
        if (e.key === "ArrowUp" || e.key === "ArrowRight")
          onCommit(Math.min(1, Math.round((v + 1 / 32) * 64) / 64));
        else if (e.key === "ArrowDown" || e.key === "ArrowLeft")
          onCommit(Math.max(-1, Math.round((v - 1 / 32) * 64) / 64));
        else return;
        e.preventDefault();
      }}
    >
      <svg viewBox="0 0 32 32" width="30" height="30" aria-hidden>
        <path d="M6.7 25.3A13 13 0 1 1 25.3 25.3" className="knob__arc" />
        <circle cx="16" cy="16" r="9" className="knob__body" />
        <line
          x1="16"
          y1="16"
          x2="16"
          y2="8.5"
          className="knob__pointer"
          transform={`rotate(${angle} 16 16)`}
        />
      </svg>
      <span className="knob__value">{formatPan(v)}</span>
    </div>
  );
}

const FADER_SCALE = [6, 0, -6, -12, -24, -40, -60];

function Slot({
  text,
  tone = "empty",
  title,
}: {
  text?: string;
  tone?: "empty" | "output" | "group" | "input" | "instrument";
  title: string;
}) {
  return (
    <div className={`slot slot--${tone}`} title={title} aria-disabled="true">
      {text}
    </div>
  );
}

const ChannelStrip = memo(function ChannelStrip({
  track,
  selected,
  silent,
  registry,
  onSelect,
}: {
  track: Track;
  selected: boolean;
  silent: boolean;
  registry: MeterRegistry;
  onSelect: (id: string) => void;
}) {
  const apply = useProject((s) => s.apply);
  const later =
    "Available with the audio engine and plug-in hosting (later phases)";
  return (
    <div
      className={`strip${selected ? " is-selected" : ""}${silent ? " is-silent" : ""}`}
      style={{ ["--track-color" as string]: track.color }}
      onPointerDown={() => onSelect(track.id)}
    >
      <Slot
        tone={track.kind === "instrument" ? "instrument" : "input"}
        text={
          track.kind === "audio"
            ? "Input"
            : track.kind === "midi"
              ? "MIDI Out"
              : "Instrument"
        }
        title={`Input — ${later}`}
      />
      <div className="strip__fx">
        {Array.from({ length: 5 }, (_, index) => (
          <Slot key={index} title={`Audio FX slot ${index + 1} — ${later}`} />
        ))}
      </div>
      <Slot
        tone="output"
        text="Stereo Out"
        title={`Output routing — ${later}`}
      />
      <Slot tone="group" title={`Group — ${later}`} />
      <PanKnob
        value={track.pan}
        onCommit={(pan) =>
          void apply({ type: "setTrackPan", id: track.id, pan })
        }
      />
      <div className="strip__db">
        <span className="strip__db-box" title="Fader level">
          {formatDb(track.volumeDb)}
        </span>
        <span
          className="strip__db-box strip__db-box--peak"
          title="Peak level — no signal without the audio engine"
        >
          −∞
        </span>
      </div>
      <div className="strip__fader-row">
        <div className="fader-scale" aria-hidden>
          {FADER_SCALE.map((db) => (
            <span key={db} style={{ bottom: `${dbToFaderPos(db) * 100}%` }}>
              {db > 0 ? `+${db}` : db}
            </span>
          ))}
        </div>
        <div
          className="meter"
          title="Level meter — no signal (audio engine offline)"
        >
          <div
            className="meter__bar"
            ref={(el) => {
              if (el) registry.set(track.id, el);
              else registry.delete(track.id);
            }}
          />
        </div>
        <Fader
          value={track.volumeDb}
          label={`${track.name} volume`}
          onCommit={(volumeDb) =>
            void apply({ type: "setTrackVolume", id: track.id, volumeDb })
          }
        />
      </div>
      <div className="strip__ms">
        <button
          type="button"
          className={`ms-btn ms-btn--mute${track.muted ? " is-on" : ""}`}
          aria-pressed={track.muted}
          onClick={() =>
            void apply({
              type: "setTracksMuted",
              ids: [track.id],
              muted: !track.muted,
            })
          }
        >
          M
        </button>
        <button
          type="button"
          className={`ms-btn ms-btn--solo${track.soloed ? " is-on" : ""}`}
          aria-pressed={track.soloed}
          onClick={() =>
            void apply({
              type: "setTracksSoloed",
              ids: [track.id],
              soloed: !track.soloed,
            })
          }
        >
          S
        </button>
      </div>
      <div className="strip__name" title={track.name}>
        <span>{track.name}</span>
      </div>
    </div>
  );
});

/**
 * Real device output meters. Values come from the native engine's lock-free
 * telemetry; there is no master bus processing yet (no fader).
 */
function OutputStrip() {
  const status = useAudio((s) => s.status);
  const telemetry = useAudio((s) => s.telemetry);
  const running = engineRunning(status);
  const peaks = running ? telemetry?.outputPeakDb : undefined;
  const hold = useAudio((s) => (running ? s.outputHoldDb : MIN_DB));
  const shown = (peaks ?? [MIN_DB, MIN_DB]).slice(0, 8);
  const clipped = (telemetry?.stats.clippedSamples ?? 0) > 0;
  const output = status?.output;
  return (
    <div
      className={`strip strip--master strip--output${running ? "" : " is-unavailable"}`}
      title={
        running && output
          ? `Device output: ${output.deviceName} (${output.host}), ${output.channels} ch. Real meters; master bus processing arrives with the audio graph.`
          : `Device output — ${status?.message ?? "audio engine not running"}`
      }
    >
      <div className="strip__db">
        <span
          className={`strip__db-box strip__db-box--peak${clipped ? " is-clipped" : ""}`}
          title={clipped ? `${telemetry?.stats.clippedSamples} samples clipped at the device output` : "Output peak (held 1.5 s)"}
          data-testid="output-peak"
        >
          {formatDb(hold)}
        </span>
      </div>
      <div className="strip__fader-row strip__fader-row--output">
        {shown.map((db, i) => (
          <div key={i} className="meter meter--live" aria-label={`Output channel ${i + 1} level`}>
            <div className="meter__bar" style={{ transform: `scaleY(${dbToFaderPos(db).toFixed(3)})` }} />
          </div>
        ))}
      </div>
      {!running && (
        <div className="strip__unavailable">
          <Icon name="lock" size={14} />
          <span>Engine offline</span>
        </div>
      )}
      <div className="strip__name">
        <span>{output ? `Out ${output.channels === 1 ? "Mono" : output.channels === 2 ? "Stereo" : `${output.channels} ch`}` : "Output"}</span>
      </div>
    </div>
  );
}

/**
 * Drives the meters with synthetic levels when the user turns on the
 * clearly-labelled simulation. Never runs otherwise (zero idle cost).
 */
function useSimulatedMeters(
  registry: MeterRegistry,
  tracks: Track[],
  on: boolean,
) {
  const tracksRef = useRef(tracks);
  useEffect(() => {
    tracksRef.current = tracks;
  }, [tracks]);
  useEffect(() => {
    if (!on) {
      registry.forEach((el) => (el.style.transform = "scaleY(0)"));
      return;
    }
    const levels = new Map<string, number>();
    let frame = 0;
    let last = 0;
    const tick = (now: number) => {
      frame = requestAnimationFrame(tick);
      if (now - last < 33) return; // ~30 fps is plenty for a simulation
      last = now;
      const ts = tracksRef.current;
      const anySolo = ts.some((t) => t.soloed);
      for (const t of ts) {
        const el = registry.get(t.id);
        if (!el) continue;
        const audible = !t.muted && (!anySolo || t.soloed);
        const target = audible
          ? dbToFaderPos(t.volumeDb) * (0.55 + Math.random() * 0.4)
          : 0;
        const prev = levels.get(t.id) ?? 0;
        const next = target > prev ? target : prev * 0.88;
        levels.set(t.id, next);
        el.style.transform = `scaleY(${next.toFixed(3)})`;
      }
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [on, registry]);
}

const KIND_FILTERS: { kind: TrackKind; label: string }[] = [
  { kind: "audio", label: "Audio" },
  { kind: "instrument", label: "Inst" },
  { kind: "midi", label: "MIDI" },
];

export function Mixer() {
  const project = useProject((s) => s.view?.project);
  const status = useAudio((s) => s.status);
  const running = engineRunning(status);
  const selection = useUi((s) => s.selection);
  const demoMeters = useUi((s) => s.demoMeters);
  const setDemoMeters = useUi((s) => s.setDemoMeters);
  const [registry] = useState<MeterRegistry>(() => new Map());
  const [hidden, setHidden] = useState<TrackKind[]>([]);
  const tracks = project?.tracks ?? [];
  // Tracks are not rendered by the engine yet, so their meters can only ever
  // show the labelled simulation, whether or not a device is open.
  useSimulatedMeters(registry, tracks, demoMeters);

  const onSelect = (id: string) => useUi.getState().selectTracks([id], id);
  const anySolo = tracks.some((t) => t.soloed);
  const shown = tracks.filter((t) => !hidden.includes(t.kind));

  return (
    <div className="mixer">
      <div className="mixer__bar">
        <span className="mixer__heading">Mixer</span>
        <span
          className="mixer__notice"
          title={
            running
              ? "The engine is running, but track playback is not implemented yet: tracks produce no signal. The output meter shows the real device output (silence or the test tone)."
              : "Faders, pan, mute and solo edit the project; there is no audio signal without a running engine."
          }
        >
          <Icon name="info" size={12} />
          <span>{running ? "Tracks not rendered yet — output meter is live" : "No signal — audio engine offline"}</span>
        </span>
        <div
          className="segmented segmented--blue"
          role="group"
          aria-label="Show track types"
        >
          {KIND_FILTERS.map((f) => {
            const on = !hidden.includes(f.kind);
            return (
              <button
                key={f.kind}
                type="button"
                className={on ? "is-on" : ""}
                aria-pressed={on}
                onClick={() =>
                  setHidden(
                    on
                      ? [...hidden, f.kind]
                      : hidden.filter((k) => k !== f.kind),
                  )
                }
              >
                {f.label}
              </button>
            );
          })}
        </div>
        <div className="mixer__bar-end">
          {demoMeters && <Badge tone="demo">Simulated</Badge>}
          <Toggle
            label="Simulate meters"
            checked={demoMeters}
            onChange={setDemoMeters}
          />
        </div>
      </div>
      <div className="mixer__body">
        {tracks.length > 0 && (
          <div className="strip-labels" aria-hidden>
            <span className="strip-labels__slot">Input</span>
            <span className="strip-labels__fx">Audio FX</span>
            <span className="strip-labels__slot">Output</span>
            <span className="strip-labels__slot">Group</span>
            <span className="strip-labels__pan">Pan</span>
            <span className="strip-labels__slot">dB</span>
          </div>
        )}
        <div className="mixer__strips">
          {tracks.length === 0 && (
            <p className="mixer__empty">
              Add tracks to see their channel strips.
            </p>
          )}
          {shown.map((t) => (
            <ChannelStrip
              key={t.id}
              track={t}
              selected={selection.trackIds.includes(t.id)}
              silent={t.muted || (anySolo && !t.soloed)}
              registry={registry}
              onSelect={onSelect}
            />
          ))}
          {tracks.length > 0 && <OutputStrip />}
        </div>
      </div>
    </div>
  );
}
