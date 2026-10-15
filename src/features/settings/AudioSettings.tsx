// Settings › Audio: device selection, stream details, test tone, live meters
// and stream health. Everything shown comes from the native engine; nothing
// here processes audio.

import { useEffect, useMemo, useState } from "react";
import type { DeviceInfo } from "../../bindings/DeviceInfo";
import type { EngineStatus } from "../../bindings/EngineStatus";
import type { ToneChannels } from "../../bindings/ToneChannels";
import { Badge, Button, Toggle } from "../../components/Button";
import { Icon } from "../../components/Icon";
import { dbToFaderPos, formatDb } from "../../lib/format";
import { ipc } from "../../lib/ipc";
import { useApp } from "../../state/app";
import { engineRunning, useAudio } from "../../state/audio";

const SYSTEM_DEFAULT = "";
const TONE_LEVELS = [-30, -24, -18, -12, -6];
const TONE_FREQUENCIES = [220, 440, 1000];

interface Draft {
  enabled: boolean;
  host: string;
  output: string;
  input: string;
  rate: string;
  buffer: string;
}

function stateLabel(status: EngineStatus | null): string {
  switch (status?.state) {
    case "running":
      return "Running";
    case "starting":
      return "Starting…";
    case "disabled":
      return "Off";
    case "noDevice":
      return "No output device";
    case "deviceLost":
      return "Device disconnected";
    case "error":
      return "Could not open the device";
    default:
      return "Unknown";
  }
}

function LevelMeter({ label, db }: { label: string; db: number }) {
  return (
    <div className="level-meter" aria-label={`${label}: ${formatDb(db)} dBFS`}>
      <span className="level-meter__label">{label}</span>
      <span className="level-meter__track">
        <span className="level-meter__bar" style={{ transform: `scaleX(${dbToFaderPos(db).toFixed(3)})` }} />
      </span>
      <span className="level-meter__value">{formatDb(db)}</span>
    </div>
  );
}

function channelName(i: number, channels: number) {
  if (channels === 2) return i === 0 ? "L" : "R";
  return `${i + 1}`;
}

function StreamFacts({ status }: { status: EngineStatus }) {
  const telemetry = useAudio((s) => s.telemetry);
  const o = status.output;
  if (!o) return null;
  const buffer =
    o.reportedBufferFrames ?? o.requestedBufferFrames
      ? `${o.reportedBufferFrames ?? o.requestedBufferFrames} frames${o.requestedBufferFrames ? "" : " (device default)"}`
      : "Device default (size reported once audio flows)";
  const measured = telemetry?.stats.measuredOutputLatencyMs;
  return (
    <dl className="facts" data-testid="audio-stream-facts">
      <dt>Audio system</dt>
      <dd>{o.host}</dd>
      <dt>Output</dt>
      <dd>
        {o.deviceName} {o.headless && <Badge tone="warning">No sound output</Badge>}
      </dd>
      <dt>Format</dt>
      <dd>
        {o.sampleRate} Hz · {o.sampleFormat} ({o.bitsPerSample}-bit) · {o.channels} ch {o.layout} · {o.bufferLayout}
      </dd>
      <dt>Buffer</dt>
      <dd>{buffer}</dd>
      <dt>Latency</dt>
      <dd>
        {o.nominalLatencyMs !== null ? `${o.nominalLatencyMs.toFixed(1)} ms per buffer` : "Unknown"}
        {measured !== null && measured !== undefined ? ` · driver-reported output delay ${measured.toFixed(1)} ms` : ""}
      </dd>
      {status.input && (
        <>
          <dt>Input (metering)</dt>
          <dd>
            {status.input.deviceName} · {status.input.sampleRate} Hz · {status.input.channels} ch {status.input.sampleFormat}
          </dd>
        </>
      )}
    </dl>
  );
}

function Health() {
  const stats = useAudio((s) => s.telemetry?.stats);
  if (!stats) return null;
  const items: [string, string, boolean][] = [
    ["Callbacks", `${stats.callbacks}`, false],
    ["Backend xruns", `${stats.backendXruns}`, stats.backendXruns > 0],
    ["Probable dropouts", `${stats.timingGaps}`, stats.timingGaps > 0],
    ["Late callbacks", `${stats.lateCallbacks}`, stats.lateCallbacks > 0],
    ["Stream errors", `${stats.streamErrors}`, stats.streamErrors > 0],
    ["Clipped samples", `${stats.clippedSamples}`, stats.clippedSamples > 0],
    ["Peak callback load", `${Math.round(stats.maxCallbackLoad * 100)}%`, stats.maxCallbackLoad > 0.8],
    ["Last block", `${stats.lastBlockFrames} frames`, false],
  ];
  return (
    <div className="audio-health" data-testid="audio-health">
      {items.map(([label, value, bad]) => (
        <div key={label} className={`audio-health__item${bad ? " is-bad" : ""}`}>
          <span>{label}</span>
          <strong>{value}</strong>
        </div>
      ))}
      {stats.realtimeDenied && <p className="muted">The system refused real-time priority for the audio thread.</p>}
    </div>
  );
}

function TestTonePanel({ status }: { status: EngineStatus }) {
  const tone = useAudio((s) => s.tone);
  const setTone = useAudio((s) => s.setTone);
  const [level, setLevel] = useState(-18);
  const [frequency, setFrequency] = useState(440);
  const [channel, setChannel] = useState("all");
  const channels = status.output?.channels ?? 2;
  const on = !!tone?.enabled;
  const target: ToneChannels = channel === "all" ? { type: "all" } : { type: "only", channel: Number(channel) };
  const send = (enabled: boolean, patch: Partial<{ level: number; frequency: number; channel: ToneChannels }> = {}) =>
    void setTone({
      enabled,
      levelDb: patch.level ?? level,
      frequencyHz: patch.frequency ?? frequency,
      channels: patch.channel ?? target,
    });
  // Never leave a tone running behind a closed dialog.
  useEffect(() => () => void useAudio.getState().setTone({ enabled: false, levelDb: -18, frequencyHz: 440, channels: { type: "all" } }), []);
  return (
    <div className="audio-tone">
      <Button variant={on ? "danger" : "primary"} icon={on ? "stop" : "play"} onClick={() => send(!on)}>
        {on ? "Stop test tone" : "Play test tone"}
      </Button>
      <label className="audio-field audio-field--inline">
        <span>Level</span>
        <select
          className="select"
          value={level}
          onChange={(e) => {
            setLevel(Number(e.target.value));
            if (on) send(true, { level: Number(e.target.value) });
          }}
        >
          {TONE_LEVELS.map((l) => (
            <option key={l} value={l}>
              {l} dBFS
            </option>
          ))}
        </select>
      </label>
      <label className="audio-field audio-field--inline">
        <span>Pitch</span>
        <select
          className="select"
          value={frequency}
          onChange={(e) => {
            setFrequency(Number(e.target.value));
            if (on) send(true, { frequency: Number(e.target.value) });
          }}
        >
          {TONE_FREQUENCIES.map((f) => (
            <option key={f} value={f}>
              {f} Hz
            </option>
          ))}
        </select>
      </label>
      <label className="audio-field audio-field--inline">
        <span>Channel</span>
        <select
          className="select"
          value={channel}
          onChange={(e) => {
            setChannel(e.target.value);
            const c: ToneChannels = e.target.value === "all" ? { type: "all" } : { type: "only", channel: Number(e.target.value) };
            if (on) send(true, { channel: c });
          }}
        >
          <option value="all">All</option>
          {Array.from({ length: Math.min(channels, 32) }, (_, i) => (
            <option key={i} value={i}>
              {channels === 2 ? (i === 0 ? "Left" : "Right") : `Channel ${i + 1}`}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}

function Meters({ status }: { status: EngineStatus }) {
  const telemetry = useAudio((s) => s.telemetry);
  const out = telemetry?.outputPeakDb ?? [];
  const inp = telemetry?.inputPeakDb ?? [];
  return (
    <div className="audio-meters" data-testid="audio-meters">
      {out.slice(0, 8).map((db, i) => (
        <LevelMeter key={`o${i}`} label={`Out ${channelName(i, out.length)}`} db={db} />
      ))}
      {status.input &&
        inp.slice(0, 8).map((db, i) => <LevelMeter key={`i${i}`} label={`In ${channelName(i, inp.length)}`} db={db} />)}
    </div>
  );
}

/** Re-reads the natively persisted device choice into the UI's settings copy. */
async function refreshAudioPrefs() {
  const fresh = await ipc.settings.get();
  useApp.setState((s) => (s.settings ? { settings: { ...s.settings, audio: fresh.audio } } : {}));
}

export function AudioSettings() {
  const status = useAudio((s) => s.status);
  const devices = useAudio((s) => s.devices);
  const loading = useAudio((s) => s.devicesLoading);
  const devicesError = useAudio((s) => s.devicesError);
  const configuring = useAudio((s) => s.configuring);
  const refreshDevices = useAudio((s) => s.refreshDevices);
  const configure = useAudio((s) => s.configure);
  const reconnect = useAudio((s) => s.reconnect);
  const prefs = useApp((s) => s.settings?.audio);

  useEffect(() => {
    void refreshDevices();
    refreshAudioPrefs().catch(() => undefined);
    return useAudio.getState().watchMeters();
  }, [refreshDevices]);

  const saved: Draft = useMemo(() => {
    const output = prefs?.outputDevice ?? SYSTEM_DEFAULT;
    const key = output || `default:${prefs?.host ?? "system"}`;
    const profile = prefs?.profiles.find((p) => p.deviceId === key);
    return {
      enabled: prefs?.enabled ?? true,
      host: prefs?.host ?? SYSTEM_DEFAULT,
      output,
      input: prefs?.inputDevice ?? SYSTEM_DEFAULT,
      rate: profile?.sampleRate ? String(profile.sampleRate) : "",
      buffer: profile?.bufferFrames ? String(profile.bufferFrames) : "",
    };
  }, [prefs]);
  const [draft, setDraft] = useState<Draft>(saved);
  // Reset the form when the saved preferences change (React's derived-state pattern).
  const [base, setBase] = useState(saved);
  if (base !== saved) {
    setBase(saved);
    setDraft(saved);
  }
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);

  const defaultHost = devices?.hosts.find((h) => h.isDefault)?.id;
  const hostId = draft.host || defaultHost;
  const outputs = (devices?.outputs ?? []).filter((d) => d.host === hostId);
  const inputs = (devices?.inputs ?? []).filter((d) => d.host === hostId);
  const selected: DeviceInfo | undefined = draft.output ? outputs.find((d) => d.id === draft.output) : outputs.find((d) => d.isDefault);
  const preferredMissing = !!draft.output && devices !== null && !outputs.some((d) => d.id === draft.output);
  const rates = selected?.sampleRates ?? [];
  const buffers = selected?.bufferSizes.length ? selected.bufferSizes : [64, 128, 256, 512, 1024, 2048];
  const update = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));

  const apply = async () => {
    const name = draft.output ? (outputs.find((d) => d.id === draft.output)?.name ?? prefs?.outputDeviceName ?? null) : null;
    const result = await configure({
      enabled: draft.enabled,
      host: draft.host || null,
      outputDevice: draft.output || null,
      outputDeviceName: name,
      inputDevice: draft.input || null,
      sampleRate: draft.rate ? Number(draft.rate) : null,
      bufferFrames: draft.buffer ? Number(draft.buffer) : null,
    });
    // The device choice is persisted natively; refresh our copy of it.
    if (result) await refreshAudioPrefs();
  };

  const running = engineRunning(status);
  const problem = status && ["noDevice", "deviceLost", "error"].includes(status.state);

  return (
    <div className="settings-page audio-settings">
      <div className={`callout ${problem ? "callout--error" : running ? "callout--ok" : "callout--info"}`} data-testid="audio-status">
        <Icon name={problem ? "warning" : running ? "check" : "info"} size={16} />
        <div className="audio-status">
          <strong>Engine: {stateLabel(status)}</strong>
          {status?.message && <p>{status.message}</p>}
          {status?.notices.map((n) => (
            <p key={n.code + n.message} className="audio-status__notice">
              {n.message}
            </p>
          ))}
          <div className="settings-actions">
            {status?.notices.some((n) => n.code === "preferredDeviceMissing") && (
              <Button size="sm" icon="refresh" disabled={configuring} onClick={() => void reconnect()}>
                Use preferred device
              </Button>
            )}
            {problem && (
              <Button size="sm" icon="refresh" disabled={configuring} onClick={() => void reconnect()}>
                Retry now
              </Button>
            )}
          </div>
        </div>
      </div>

      {running && status && <StreamFacts status={status} />}

      <h3>Device</h3>
      <Toggle
        label="Audio engine"
        description="Open the output device when Elyvon Studio starts. Turn off to release the device for other applications."
        checked={draft.enabled}
        onChange={(enabled) => update({ enabled })}
      />
      <div className="audio-form">
        <label className="audio-field">
          <span>Audio system</span>
          <select className="select" value={draft.host} onChange={(e) => update({ host: e.target.value, output: "", input: "", rate: "", buffer: "" })}>
            <option value="">System default{defaultHost ? ` (${devices?.hosts.find((h) => h.id === defaultHost)?.name})` : ""}</option>
            {devices?.hosts.map((h) => (
              <option key={h.id} value={h.id}>
                {h.name}
                {h.headless ? " — no sound output" : ""}
              </option>
            ))}
          </select>
        </label>
        <label className="audio-field">
          <span>Output device</span>
          <select className="select" value={draft.output} onChange={(e) => update({ output: e.target.value, rate: "", buffer: "" })}>
            <option value="">System default output{outputs.find((d) => d.isDefault) ? ` (${outputs.find((d) => d.isDefault)?.name})` : ""}</option>
            {preferredMissing && <option value={draft.output}>{prefs?.outputDeviceName ?? draft.output} — not connected</option>}
            {outputs.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </select>
        </label>
        <label className="audio-field">
          <span>Sample rate</span>
          <select className="select" value={draft.rate} onChange={(e) => update({ rate: e.target.value })}>
            <option value="">Device default{selected?.defaultConfig ? ` (${selected.defaultConfig.sampleRate} Hz)` : ""}</option>
            {rates.map((r) => (
              <option key={r} value={r}>
                {r} Hz
              </option>
            ))}
          </select>
        </label>
        <label className="audio-field">
          <span>Buffer size</span>
          <select className="select" value={draft.buffer} onChange={(e) => update({ buffer: e.target.value })}>
            <option value="">Device default</option>
            {buffers.map((b) => (
              <option key={b} value={b}>
                {b} frames{selected?.defaultConfig ? ` (${((b / (Number(draft.rate) || selected.defaultConfig.sampleRate)) * 1000).toFixed(1)} ms)` : ""}
              </option>
            ))}
          </select>
        </label>
        <label className="audio-field">
          <span>Input (level meter only)</span>
          <select className="select" value={draft.input} onChange={(e) => update({ input: e.target.value })}>
            <option value="">None</option>
            {inputs.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      {selected && (
        <p className="muted" data-testid="audio-device-details">
          {selected.name}
          {selected.manufacturer ? ` · ${selected.manufacturer}` : ""}: up to {selected.maxChannels} channels
          {selected.defaultConfig
            ? `, default ${selected.defaultConfig.channels} ch ${selected.defaultConfig.sampleRate} Hz ${selected.defaultConfig.sampleFormat} (${selected.defaultConfig.bitsPerSample}-bit)`
            : ""}
          {selected.bufferSizes.length === 0 ? ". The driver does not report its buffer range; unsupported sizes fall back to the default." : "."}
          {selected.ranges.some((r) => !r.supportedByEngine) ? " Some formats (24-bit packed or DSD) are not used by the engine." : ""}
          {selected.error ? ` ${selected.error}` : ""}
        </p>
      )}
      {devicesError && (
        <div className="callout callout--error">
          <Icon name="warning" size={16} />
          <p>Could not list devices: {devicesError.message}</p>
        </div>
      )}
      {devices?.errors.map((e) => (
        <p key={e} className="muted">
          {e}
        </p>
      ))}
      <div className="settings-actions">
        <Button variant="primary" disabled={!dirty || configuring} onClick={() => void apply()}>
          {configuring ? "Applying…" : "Apply"}
        </Button>
        <Button icon="refresh" disabled={loading} onClick={() => void refreshDevices()}>
          {loading ? "Scanning…" : "Rescan devices"}
        </Button>
      </div>

      {running && status && (
        <>
          <h3>Test output</h3>
          <p className="muted">
            Plays a sine tone through the selected output. The level never exceeds −6 dBFS; turn your monitors down before testing.
          </p>
          <TestTonePanel status={status} />
          <Meters status={status} />
          <h3>Stream health</h3>
          <Health />
        </>
      )}
      <p className="muted">
        Sample rate and buffer size are remembered per device. If a preferred device is missing, Elyvon uses the system default and tells you;
        unplugging a device stops playback without touching your project.
      </p>
    </div>
  );
}
