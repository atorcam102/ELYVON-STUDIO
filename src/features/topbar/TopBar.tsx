import { useState } from "react";
import type { KeyMode } from "../../bindings/KeyMode";
import type { PitchClass } from "../../bindings/PitchClass";
import { Badge, IconButton } from "../../components/Button";
import { InlineEdit, NumberField } from "../../components/Fields";
import { Icon } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
import { DENOMINATORS, formatClock, formatKey, positionFromTicks, PITCH_CLASSES } from "../../lib/music";
import { shortcutLabel } from "../../lib/platform";
import { engineAvailable, useApp } from "../../state/app";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { hasUpdateBadge, useUpdates } from "../../state/updates";
import { redo, toggleLoop, undo } from "../project/editActions";
import { newProject, openProject, saveProject } from "../project/projectActions";

function BrandMark() {
  return (
    <div className="brand" title="Elyvon Studio">
      <svg viewBox="0 0 32 32" width="22" height="22" aria-hidden>
        <defs>
          <linearGradient id="brand-g" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="#7FE3D0" />
            <stop offset=".55" stopColor="#5B8DEF" />
            <stop offset="1" stopColor="#9B7BE8" />
          </linearGradient>
        </defs>
        <rect x="1" y="1" width="30" height="30" rx="8" fill="#1A1C23" />
        <g fill="url(#brand-g)">
          <rect x="9" y="8" width="3.2" height="16" rx="1.6" />
          <rect x="9" y="8" width="14" height="3.2" rx="1.6" />
          <rect x="9" y="14.4" width="10.5" height="3.2" rx="1.6" />
          <rect x="9" y="20.8" width="14" height="3.2" rx="1.6" />
        </g>
      </svg>
      <span className="brand__name">
        Elyvon <span className="brand__product">Studio</span>
      </span>
    </div>
  );
}

function Transport() {
  const view = useProject((s) => s.view);
  const engine = useApp((s) => s.engine);
  const setCursor = useUi((s) => s.setCursor);
  const available = engineAvailable(engine);
  const reason = engine?.state === "unavailable" ? engine.reason : "";
  const loop = view?.project.transport.loopRange.enabled ?? false;
  const unavailable = (what: string) => `${what} — not available: ${reason}`;
  return (
    <div className="transport" role="group" aria-label="Transport">
      <IconButton icon="toStart" label="Go to beginning" onClick={() => setCursor(0)} />
      <IconButton icon="stop" label={available ? "Stop" : unavailable("Stop")} disabled={!available} />
      <IconButton icon="play" label={available ? "Play" : unavailable("Play")} disabled={!available} size={17} />
      <IconButton
        icon="record"
        tone="record"
        label={available ? "Record" : unavailable("Record")}
        disabled={!available}
      />
      <IconButton icon="loop" label={loop ? "Loop on" : "Loop off"} active={loop} onClick={toggleLoop} disabled={!view} tone="accent" />
    </div>
  );
}

function PositionDisplay() {
  const cursor = useUi((s) => s.cursorTicks);
  const transport = useProject((s) => s.view?.project.transport);
  if (!transport) return <div className="lcd" />;
  const p = positionFromTicks(cursor, transport.timeSignature);
  return (
    <div className="lcd" title="Edit cursor position (bar · beat · division · tick) and elapsed time">
      <div className="lcd__primary">
        <span>{String(p.bar).padStart(3, " ")}</span>
        <i>.</i>
        <span>{p.beat}</span>
        <i>.</i>
        <span>{p.division}</span>
        <i>.</i>
        <span className="lcd__dim">{String(p.tick).padStart(3, "0")}</span>
      </div>
      <div className="lcd__secondary">{formatClock(cursor, transport.tempo)}</div>
    </div>
  );
}

function SongSettings() {
  const transport = useProject((s) => s.view?.project.transport);
  const apply = useProject((s) => s.apply);
  const sigMenu = useMenu();
  const keyMenu = useMenu();
  if (!transport) return null;
  const ts = transport.timeSignature;

  const sigEntries: MenuEntry[] = [
    { heading: "Common" },
    ...[
      [4, 4],
      [3, 4],
      [2, 4],
      [6, 8],
      [5, 4],
      [7, 8],
      [12, 8],
    ].map(([n, d]) => ({
      id: `${n}/${d}`,
      label: `${n}/${d}`,
      onSelect: () => void apply({ type: "setTimeSignature", numerator: n!, denominator: d! }),
    })),
    "separator",
    {
      key: "custom",
      custom: (
        <div className="sig-editor">
          <NumberField
            label="Beats per bar"
            value={ts.numerator}
            min={1}
            max={32}
            onCommit={(numerator) => void apply({ type: "setTimeSignature", numerator, denominator: ts.denominator })}
          />
          <span>/</span>
          <select
            aria-label="Beat unit"
            value={ts.denominator}
            onChange={(e) => void apply({ type: "setTimeSignature", numerator: ts.numerator, denominator: Number(e.target.value) })}
          >
            {DENOMINATORS.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
        </div>
      ),
    },
  ];

  const setKey = (tonic: PitchClass, mode: KeyMode) => void apply({ type: "setKey", key: { tonic, mode } });
  const keyEntries: MenuEntry[] = [
    {
      key: "keys",
      custom: (
        <div className="key-grid">
          {(["major", "minor"] as const).map((mode) => (
            <div key={mode} className="key-grid__col">
              <div className="menu__heading">{mode === "major" ? "Major" : "Minor"}</div>
              <div className="key-grid__cells">
                {PITCH_CLASSES.map((pc) => (
                  <button
                    key={pc}
                    type="button"
                    className={`key-grid__cell${transport.key.tonic === pc && transport.key.mode === mode ? " is-active" : ""}`}
                    onClick={() => {
                      setKey(pc, mode);
                      keyMenu.close();
                    }}
                  >
                    {mode === "minor" ? pc.toLowerCase() : pc}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </div>
      ),
    },
  ];

  return (
    <div className="song-settings">
      <label className="field-chip" title="Tempo (beats per minute)">
        <NumberField
          label="Tempo"
          value={transport.tempo}
          min={20}
          max={999}
          step={1}
          precision={transport.tempo % 1 === 0 ? 0 : 2}
          onCommit={(bpm) => void apply({ type: "setTempo", bpm })}
          className="field-chip__number"
        />
        <span className="field-chip__unit">BPM</span>
      </label>
      <button type="button" className="field-chip" title="Time signature" onClick={(e) => sigMenu.openBelow(e.currentTarget)}>
        <span className="field-chip__value">{`${ts.numerator}/${ts.denominator}`}</span>
        <Icon name="chevronDown" size={12} />
      </button>
      <button type="button" className="field-chip" title="Key" onClick={(e) => keyMenu.openBelow(e.currentTarget)}>
        <span className="field-chip__value">{formatKey(transport.key)}</span>
        <Icon name="chevronDown" size={12} />
      </button>
      {sigMenu.at && <Menu at={sigMenu.at} entries={sigEntries} onClose={sigMenu.close} />}
      {keyMenu.at && <Menu at={keyMenu.at} entries={keyEntries} onClose={keyMenu.close} />}
    </div>
  );
}

function ProjectTitle() {
  const view = useProject((s) => s.view);
  const apply = useProject((s) => s.apply);
  const [editing, setEditing] = useState(false);
  if (!view) return <div className="project-title" />;
  const where = view.path ?? (view.origin === "demo" ? "Demo session — not saved" : "Not saved yet");
  return (
    <div className="project-title" title={where}>
      <div className="project-title__row">
        <InlineEdit
          value={view.project.name}
          label="Project name"
          editing={editing}
          onEditingChange={setEditing}
          onCommit={(name) => void apply({ type: "renameProject", name })}
          className="project-title__name"
        />
        {view.dirty && <span className="dirty-dot" title="Unsaved changes" aria-label="Unsaved changes" />}
        {view.origin === "demo" && <Badge tone="demo">Demo</Badge>}
        {view.origin === "recovered" && <Badge tone="warning">Recovered</Badge>}
      </div>
      <div className="project-title__path">{where}</div>
    </div>
  );
}

function StatusIndicator() {
  const engine = useApp((s) => s.engine);
  const pending = useProject((s) => s.pending);
  const updates = useUpdates((s) => s.state);
  const setUpdateOpen = useUi((s) => s.setUpdateOpen);
  return (
    <div className="status">
      {hasUpdateBadge(updates) && (
        <button type="button" className="status__update" onClick={() => setUpdateOpen(true)} title="An update is available">
          <Icon name="download" size={13} />
          <span>Update</span>
        </button>
      )}
      <div
        className={`status__pill ${engineAvailable(engine) ? "is-ok" : "is-offline"}`}
        title={engine?.state === "unavailable" ? engine.reason : "Audio engine running"}
      >
        <span className="status__dot" />
        <span>{engineAvailable(engine) ? "Engine running" : "Audio engine offline"}</span>
      </div>
      {pending > 0 && <span className="status__busy" aria-label="Working" />}
    </div>
  );
}

export function TopBar() {
  const view = useProject((s) => s.view);
  const openSettings = useUi((s) => s.openSettings);
  return (
    <header className="topbar" data-tauri-drag-region>
      <BrandMark />
      <div className="topbar__group">
        <IconButton icon="newFile" label={`New project (${shortcutLabel("Mod+N")})`} onClick={() => void newProject()} />
        <IconButton icon="open" label={`Open project (${shortcutLabel("Mod+O")})`} onClick={() => void openProject()} />
        <IconButton icon="save" label={`Save (${shortcutLabel("Mod+S")})`} onClick={() => void saveProject()} disabled={!view} />
      </div>
      <div className="topbar__group">
        <IconButton
          icon="undo"
          label={view?.undoLabel ? `Undo ${view.undoLabel} (${shortcutLabel("Mod+Z")})` : "Nothing to undo"}
          disabled={!view?.undoLabel}
          onClick={undo}
        />
        <IconButton
          icon="redo"
          label={view?.redoLabel ? `Redo ${view.redoLabel} (${shortcutLabel("Mod+Shift+Z")})` : "Nothing to redo"}
          disabled={!view?.redoLabel}
          onClick={redo}
        />
      </div>
      <ProjectTitle />
      <div className="topbar__center">
        <Transport />
        <PositionDisplay />
        <SongSettings />
      </div>
      <div className="topbar__end">
        <StatusIndicator />
        <IconButton icon="settings" label="Settings" onClick={() => openSettings()} />
      </div>
    </header>
  );
}
