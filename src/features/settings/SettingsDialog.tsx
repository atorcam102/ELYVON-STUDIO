import { Button, Toggle } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Icon } from "../../components/Icon";
import { fileName } from "../../lib/format";
import { isMac, shortcutLabel } from "../../lib/platform";
import { useApp } from "../../state/app";
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
  return (
    <div className="settings-page">
      <div className="callout callout--info">
        <Icon name="info" size={16} />
        <p>{engine?.state === "unavailable" ? engine.reason : "Audio engine running."}</p>
      </div>
      <p className="muted">
        Device selection, sample rate and buffer size will appear here with the audio engine. The engine will run on a dedicated real-time thread in
        the native core, separate from the interface and from disk access.
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
  ["Toggle loop", "L"],
  ["Cursor to start", "Home"],
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
