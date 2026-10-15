import { useState } from "react";
import type { KeyMode } from "../../bindings/KeyMode";
import type { PitchClass } from "../../bindings/PitchClass";
import { Badge } from "../../components/Button";
import { InlineEdit, NumberField } from "../../components/Fields";
import { Icon, type IconName } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
import { DENOMINATORS, formatClock, positionFromTicks, PITCH_CLASSES } from "../../lib/music";
import { shortcutLabel } from "../../lib/platform";
import { engineAvailable, useApp } from "../../state/app";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { hasUpdateBadge, useUpdates } from "../../state/updates";
import { redo, toggleLoop, undo } from "../project/editActions";
import { newProject, openProject, saveProject } from "../project/projectActions";

interface CbButtonProps {
  icon: IconName;
  label: string;
  onClick?: () => void;
  active?: boolean;
  disabled?: boolean;
  tone?: "default" | "play" | "record" | "cycle" | "view";
  size?: number;
}

/** Control-bar button: borderless, rounded, with signal colours per tone. */
function CbButton({ icon, label, onClick, active, disabled, tone = "default", size = 16 }: CbButtonProps) {
  return (
    <button
      type="button"
      className={`cb-btn cb-btn--${tone}${active ? " is-active" : ""}`}
      aria-label={label}
      title={label}
      aria-pressed={active === undefined ? undefined : active}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon name={icon} size={size} />
    </button>
  );
}

function ViewButtons() {
  const layout = useUi((s) => s.layout);
  const setLayout = useUi((s) => s.setLayout);
  const mixerShown = layout.bottomVisible && layout.bottomTab === "mixer";
  const editorShown = layout.bottomVisible && layout.bottomTab !== "mixer";
  return (
    <div className="cb-group" role="group" aria-label="View">
      <CbButton
        icon="panelLeft"
        tone="view"
        label="Inspector (I)"
        active={layout.inspectorVisible}
        onClick={() => setLayout({ inspectorVisible: !layout.inspectorVisible })}
      />
      <CbButton
        icon="mixer"
        tone="view"
        label="Mixer (X)"
        active={mixerShown}
        onClick={() => setLayout(mixerShown ? { bottomVisible: false } : { bottomVisible: true, bottomTab: "mixer" })}
      />
      <CbButton
        icon="piano"
        tone="view"
        label="Editors (E)"
        active={editorShown}
        onClick={() =>
          setLayout(
            editorShown
              ? { bottomVisible: false }
              : { bottomVisible: true, bottomTab: layout.bottomTab === "mixer" ? "pianoRoll" : layout.bottomTab },
          )
        }
      />
    </div>
  );
}

function FileButtons() {
  const view = useProject((s) => s.view);
  return (
    <div className="cb-group" role="group" aria-label="Project">
      <CbButton icon="newFile" label={`New project (${shortcutLabel("Mod+N")})`} onClick={() => void newProject()} />
      <CbButton icon="open" label={`Open project (${shortcutLabel("Mod+O")})`} onClick={() => void openProject()} />
      <CbButton icon="save" label={`Save (${shortcutLabel("Mod+S")})`} onClick={() => void saveProject()} disabled={!view} />
      <span className="cb-sep" />
      <CbButton
        icon="undo"
        label={view?.undoLabel ? `Undo ${view.undoLabel} (${shortcutLabel("Mod+Z")})` : "Nothing to undo"}
        disabled={!view?.undoLabel}
        onClick={undo}
      />
      <CbButton
        icon="redo"
        label={view?.redoLabel ? `Redo ${view.redoLabel} (${shortcutLabel("Mod+Shift+Z")})` : "Nothing to redo"}
        disabled={!view?.redoLabel}
        onClick={redo}
      />
    </div>
  );
}

function Transport() {
  const engine = useApp((s) => s.engine);
  const setCursor = useUi((s) => s.setCursor);
  const available = engineAvailable(engine);
  const reason = engine?.state === "unavailable" ? engine.reason : "";
  const label = (what: string) => (available ? what : `${what} — unavailable: ${reason}`);
  return (
    <div className="cb-transport" role="group" aria-label="Transport">
      <CbButton icon="toStart" label="Go to beginning" onClick={() => setCursor(0)} />
      <CbButton icon="stop" label={label("Stop")} disabled={!available} />
      <CbButton icon="play" tone="play" label={label("Play")} disabled={!available} size={17} />
      <CbButton icon="record" tone="record" label={label("Record")} disabled={!available} />
    </div>
  );
}

function Lcd() {
  const cursor = useUi((s) => s.cursorTicks);
  const transport = useProject((s) => s.view?.project.transport);
  const apply = useProject((s) => s.apply);
  const sigMenu = useMenu();
  const keyMenu = useMenu();
  if (!transport) return <div className="lcd" />;
  const ts = transport.timeSignature;
  const p = positionFromTicks(cursor, ts);

  const sigEntries: MenuEntry[] = [
    { heading: "Time signature" },
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
                    {mode === "minor" ? `${pc}m` : pc}
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
    <div className="lcd" role="group" aria-label="Display">
      <div className="lcd__cell lcd__position" title={`Cursor position · ${formatClock(cursor, transport.tempo)}`}>
        <div className="lcd__digits">
          <span className="lcd__big">{p.bar}</span>
          <span className="lcd__big">{p.beat}</span>
          <span className="lcd__small">{p.division}</span>
          <span className="lcd__small">{String(p.tick).padStart(3, "0")}</span>
        </div>
        <div className="lcd__labels">
          <span>BAR</span>
          <span>BEAT</span>
          <span>DIV</span>
          <span>TICK</span>
        </div>
      </div>
      <div className="lcd__cell lcd__tempo">
        <NumberField
          label="Tempo"
          value={transport.tempo}
          min={20}
          max={999}
          precision={transport.tempo % 1 === 0 ? 0 : 2}
          onCommit={(bpm) => void apply({ type: "setTempo", bpm })}
          className="lcd__number"
        />
        <div className="lcd__labels">
          <span>TEMPO</span>
        </div>
      </div>
      <div className="lcd__cell lcd__meta">
        <button type="button" className="lcd__btn" title="Time signature" onClick={(e) => sigMenu.openBelow(e.currentTarget)}>
          {`${ts.numerator}/${ts.denominator}`}
        </button>
        <button type="button" className="lcd__btn lcd__btn--sub" title="Key" onClick={(e) => keyMenu.openBelow(e.currentTarget)}>
          {`${transport.key.tonic}${transport.key.mode === "major" ? "maj" : "min"}`}
        </button>
      </div>
      {sigMenu.at && <Menu at={sigMenu.at} entries={sigEntries} onClose={sigMenu.close} />}
      {keyMenu.at && <Menu at={keyMenu.at} entries={keyEntries} onClose={keyMenu.close} />}
    </div>
  );
}

function Modes() {
  const view = useProject((s) => s.view);
  const loop = view?.project.transport.loopRange.enabled ?? false;
  return (
    <div className="cb-group" role="group" aria-label="Modes">
      <CbButton icon="loop" tone="cycle" label={loop ? "Cycle on (L)" : "Cycle off (L)"} active={loop} onClick={toggleLoop} disabled={!view} />
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
      {pending > 0 && <span className="status__busy" aria-label="Working" />}
      {hasUpdateBadge(updates) && (
        <button type="button" className="status__update" onClick={() => setUpdateOpen(true)} title="An update is available">
          <Icon name="download" size={12} />
          <span>Update</span>
        </button>
      )}
      <div
        className={`status__pill ${engineAvailable(engine) ? "is-ok" : "is-offline"}`}
        title={engine?.state === "unavailable" ? engine.reason : "Audio engine running"}
      >
        <span className="status__dot" />
        <span className="status__text">{engineAvailable(engine) ? "Engine" : "Engine offline"}</span>
      </div>
    </div>
  );
}

export function TopBar() {
  const layout = useUi((s) => s.layout);
  const setLayout = useUi((s) => s.setLayout);
  const openSettings = useUi((s) => s.openSettings);
  return (
    <header className="controlbar">
      <div className="controlbar__side">
        <ViewButtons />
        <FileButtons />
        <ProjectTitle />
      </div>
      <div className="controlbar__center">
        <Transport />
        <Lcd />
        <Modes />
      </div>
      <div className="controlbar__side controlbar__side--end">
        <StatusIndicator />
        <div className="cb-group" role="group" aria-label="Panels">
          <CbButton
            icon="panelRight"
            tone="view"
            label="Browser (B)"
            active={layout.browserVisible}
            onClick={() => setLayout({ browserVisible: !layout.browserVisible })}
          />
          <CbButton icon="settings" label={`Settings (${shortcutLabel("Mod+,")})`} onClick={() => openSettings()} />
        </div>
      </div>
    </header>
  );
}
