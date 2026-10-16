import { useEffect, useState } from "react";
import type { DeviceInfo } from "../../bindings/DeviceInfo";
import { Button, Toggle } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Icon } from "../../components/Icon";
import { fileName } from "../../lib/format";
import { isMac, shortcutLabel } from "../../lib/platform";
import { inTauri, ipc } from "../../lib/ipc";
import { useApp } from "../../state/app";
import { notifyError } from "../../state/notifications";
import { useTransport } from "../../state/transport";
import { useUi, type SettingsSection } from "../../state/ui";
import { useUpdates } from "../../state/updates";
import { openProject } from "../project/projectActions";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "general", label: "General" },
  { id: "updates", label: "Updates" },
  { id: "audio", label: "Audio" },
  { id: "shortcuts", label: "Shortcuts" },
];

function General() {
  const settings = useApp((s) => s.settings);
  const update = useApp((s) => s.updateSettings);
  const close = useUi((s) => s.closeSettings);
  if (!settings) return null;
  return (
    <div className="settings-page">
      <h3>Recent projects</h3>
      {settings.recentProjects.length === 0 ? (
        <p className="muted">Projects you open or save appear here.</p>
      ) : (
        <ul className="recent-list">
          {settings.recentProjects.map((p) => (
            <li key={p}>
              <button
                type="button"
                className="recent-list__item"
                title={p}
                onClick={() => {
                  close();
                  void openProject(p);
                }}
              >
                <Icon name="project" size={14} />
                <span className="recent-list__name">{fileName(p)}</span>
                <span className="recent-list__path">{p}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {settings.recentProjects.length > 0 && (
        <Button size="sm" onClick={() => update((s) => ({ ...s, recentProjects: [] }))}>
          Clear list
        </Button>
      )}
      <h3>Privacy</h3>
      <p className="muted">
        Elyvon Studio collects no telemetry. The only network request it makes is the update check, which you control under Updates.
      </p>
    </div>
  );
}

function Updates() {
  const settings = useApp((s) => s.settings);
  const info = useApp((s) => s.info);
  const update = useApp((s) => s.updateSettings);
  const updates = useUpdates();
  const setUpdateOpen = useUi((s) => s.setUpdateOpen);
  if (!settings || !info) return null;
  const env = info.updater;
  return (
    <div className="settings-page">
      <dl className="facts">
        <dt>Installed version</dt>
        <dd>{info.version}</dd>
        <dt>Package</dt>
        <dd>
          {env.platformLabel ?? `${info.os} ${info.arch}`}
          {env.installer ? ` · ${env.installer}` : ""}
        </dd>
        <dt>Update channel</dt>
        <dd>Stable (GitHub Releases)</dd>
        <dt>Signature verification</dt>
        <dd>{env.pubkeyConfigured ? "Required (minisign public key embedded)" : "Not configured in this build"}</dd>
      </dl>
      {env.unavailableReason && (
        <div className="callout callout--info">
          <Icon name="info" size={16} />
          <p>{env.unavailableReason}</p>
        </div>
      )}
      <Toggle
        label="Check for updates at start-up"
        description="One background request a few seconds after launch. Nothing is downloaded or installed without your confirmation."
        checked={settings.updates.checkOnStartup}
        onChange={(checkOnStartup) => update((s) => ({ ...s, updates: { ...s.updates, checkOnStartup } }))}
      />
      <div className="settings-actions">
        <Button
          variant="primary"
          icon="refresh"
          disabled={updates.state.phase === "checking"}
          onClick={() => {
            setUpdateOpen(true);
            void updates.check();
          }}
        >
          Check for updates now
        </Button>
      </div>
    </div>
  );
}

function Audio() {
  const engine = useApp((s) => s.engine);
  const settings = useApp((s) => s.settings);
  const update = useApp((s) => s.updateSettings);
  const stats = useTransport((s) => s.stats);
  const [devices, setDevices] = useState<DeviceInfo[] | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!inTauri) return;
    ipc.audio.devices().then(setDevices, () => setDevices([]));
  }, []);
  if (!settings) return null;
  const a = settings.audio;
  const apply = async () => {
    setBusy(true);
    try {
      await ipc.settings.set(useApp.getState().settings!);
      useApp.setState({ engine: await ipc.audio.restart() });
    } catch (e) {
      notifyError(e, "Could not restart audio");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="settings-page">
      <div className={`callout ${engine?.state === "running" && engine.output === "device" ? "callout--info" : "callout--warning"}`}>
        <Icon name={engine?.state === "running" && engine.output === "device" ? "info" : "warning"} size={16} />
        <p data-testid="audio-status">
          {engine?.state === "running"
            ? `${engine.device} · ${engine.config.sampleRate} Hz · ${engine.config.outputChannels} ch${engine.config.bufferFrames ? ` · ${engine.config.bufferFrames} frames` : " · default buffer"}${
                engine.output === "internalClock" ? ". No sound is produced: the transport runs on the engine's internal clock." : ""
              }`
            : (engine?.reason ?? "The audio engine is starting.")}
        </p>
      </div>
      <Toggle
        checked={a.useDevice}
        onChange={(useDevice) => update((s) => ({ ...s, audio: { ...s.audio, useDevice } }))}
        label="Use an audio output device"
        description="Off: silent internal clock (useful without hardware)."
      />
      <label className="settings-field">
        <span>Output device</span>
        <select
          value={a.outputDevice ?? ""}
          disabled={!a.useDevice}
          onChange={(e) => update((s) => ({ ...s, audio: { ...s.audio, outputDevice: e.target.value || null } }))}
        >
          <option value="">System default</option>
          {(devices ?? []).map((d) => (
            <option key={d.id} value={d.id}>
              {d.name}
              {d.isDefault ? " (default)" : ""}
            </option>
          ))}
        </select>
      </label>
      {devices !== null && devices.length === 0 && <p className="muted">No output devices were found.</p>}
      <label className="settings-field">
        <span>Buffer size</span>
        <select
          value={a.bufferFrames}
          disabled={!a.useDevice}
          onChange={(e) => update((s) => ({ ...s, audio: { ...s.audio, bufferFrames: Number(e.target.value) } }))}
        >
          {[0, 64, 128, 256, 512, 1024, 2048].map((n) => (
            <option key={n} value={n}>
              {n === 0 ? "Device default" : `${n} frames`}
            </option>
          ))}
        </select>
      </label>
      <Button onClick={() => void apply()} disabled={busy}>
        {busy ? "Restarting…" : "Apply and restart audio"}
      </Button>
      {stats && (
        <p className="muted">
          {`Callbacks: ${stats.blocks} · last ${stats.lastBlockUs.toFixed(0)} µs · max ${stats.maxBlockUs.toFixed(0)} µs · xruns: ${stats.xruns} · disk misses: ${stats.streamingUnderruns} · MIDI overflows: ${stats.midiOverflow}`}
        </p>
      )}
      <p className="muted">
        The project sample rate is set in Tempo, metre and locators. Audio runs in the native engine on the device thread, with no locks or
        allocation in the callback; files are decoded and resampled on a worker thread.
      </p>
    </div>
  );
}

const SHORTCUTS: [string, string][] = [
  ["New project", "Mod+N"],
  ["Open project", "Mod+O"],
  ["Save", "Mod+S"],
  ["Save as", "Mod+Shift+S"],
  ["Undo", "Mod+Z"],
  ["Redo", "Mod+Shift+Z"],
  ["Settings", "Mod+,"],
  ["New audio track", "Mod+Alt+A"],
  ["New instrument track", "Mod+Alt+S"],
  ["New MIDI track", "Mod+Alt+X"],
  ["Duplicate tracks", "Mod+D"],
  ["Delete selection", "Delete"],
  ["Zoom in / out", "Mod+= / Mod+-"],
  ["Play / stop", "Space"],
  ["Toggle cycle", "L"],
  ["Metronome", "K"],
  ["Playhead to start", "Home"],
  ["Import audio files", "Mod+Shift+I"],
  ["Split regions at playhead", "Mod+T"],
  ["Cut / copy / paste regions", "Mod+X / Mod+C / Mod+V"],
  ["Mute selected regions", "M"],
  ["Toggle inspector / browser / editors", "I / B / E"],
  ["Show mixer", "X"],
  ["Select previous / next track", "↑ / ↓"],
];

function Shortcuts() {
  return (
    <div className="settings-page">
      <table className="shortcut-table">
        <tbody>
          {SHORTCUTS.map(([label, keys]) => (
            <tr key={label}>
              <td>{label}</td>
              <td>
                <kbd>{keys.split(" / ").map(shortcutLabel).join(" / ")}</kbd>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="muted">{isMac ? "⌘ is the Command key." : "Ctrl is the primary modifier on Windows and Linux."}</p>
    </div>
  );
}

export function SettingsDialog() {
  const section = useUi((s) => s.settingsOpen);
  const open = useUi((s) => s.openSettings);
  const close = useUi((s) => s.closeSettings);
  return (
    <Dialog open={section !== null} title="Settings" onClose={close} width={720} className="settings-dialog">
      <div className="settings">
        <nav className="settings__nav" aria-label="Settings sections">
          {SECTIONS.map((s) => (
            <button key={s.id} type="button" className={`settings__nav-item${section === s.id ? " is-active" : ""}`} onClick={() => open(s.id)}>
              {s.label}
            </button>
          ))}
        </nav>
        <div className="settings__content">
          {section === "general" && <General />}
          {section === "updates" && <Updates />}
          {section === "audio" && <Audio />}
          {section === "shortcuts" && <Shortcuts />}
        </div>
      </div>
    </Dialog>
  );
}
