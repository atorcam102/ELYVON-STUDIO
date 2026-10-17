import { useMidi } from "../../state/midi";
import { AppMenu } from "./AppMenu";
import { pendingControl } from "./menuActions";
import { useChrome } from "../../state/chrome";
import { MidiDialog } from "../midi/MidiDialog";
import { useCapture } from "../../state/capture";
import { RecordingDialog } from "../recording/RecordingDialog";
import { useState } from "react";
import type { KeyMode } from "../../bindings/KeyMode";
import type { PitchClass } from "../../bindings/PitchClass";
import { Badge } from "../../components/Button";
import { InlineEdit, NumberField } from "../../components/Fields";
import { Icon, type IconName } from "../../components/Icon";
import { Menu, type MenuEntry } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
import {
  DENOMINATORS,
  positionFromTicks,
  PITCH_CLASSES,
} from "../../lib/music";
import { shortcutLabel } from "../../lib/platform";
import { engineAvailable, useApp } from "../../state/app";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { hasUpdateBadge, useUpdates } from "../../state/updates";
import { redo, toggleLoop, undo } from "../project/editActions";
import { formatClockSeconds, TempoMap } from "../../lib/tempoMap";
import { isRolling, useTransport } from "../../state/transport";
import { locate, pause, play, stop, tapTempo, toggleCountIn, toggleMetronome } from "../transport/transportActions";
import { TimelineSettingsDialog } from "./TimelineSettingsDialog";
import {
  newProject,
  openProject,
  saveProject,
} from "../project/projectActions";

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
function CbButton({
  icon,
  label,
  onClick,
  active,
  disabled,
  tone = "default",
  size = 16,
}: CbButtonProps) {
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
        icon="smartControls"
        tone="view"
        label="Smart Controls (B)"
        active={layout.bottomVisible && layout.bottomTab === "smartControls"}
        onClick={() => setLayout({ bottomVisible: !(layout.bottomVisible && layout.bottomTab === "smartControls"), bottomTab: "smartControls" })}
      />
      <CbButton
        icon="sliders"
        tone="view"
        label="Mezclador (X)"
        active={mixerShown}
        onClick={() =>
          setLayout(
            mixerShown
              ? { bottomVisible: false }
              : { bottomVisible: true, bottomTab: "mixer", bottomHeight: window.innerHeight - 180 },
          )
        }
      />
      <CbButton
        icon="pencil"
        tone="view"
        label="Editor (E)"
        active={editorShown}
        onClick={() =>
          setLayout(
            editorShown
              ? { bottomVisible: false }
              : {
                  bottomVisible: true,
                  bottomHeight: Math.round(window.innerHeight * .66),
                  bottomTab:
                    layout.bottomTab === "mixer"
                      ? "pianoRoll"
                      : layout.bottomTab,
                },
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
      <CbButton
        icon="newFile"
        label={`New project (${shortcutLabel("Mod+N")})`}
        onClick={() => void newProject()}
      />
      <CbButton
        icon="open"
        label={`Open project (${shortcutLabel("Mod+O")})`}
        onClick={() => void openProject()}
      />
      <CbButton
        icon="save"
        label={`Save (${shortcutLabel("Mod+S")})`}
        onClick={() => void saveProject()}
        disabled={!view}
      />
      <span className="cb-sep" />
      <CbButton
        icon="undo"
        label={
          view?.undoLabel
            ? `Undo ${view.undoLabel} (${shortcutLabel("Mod+Z")})`
            : "Nothing to undo"
        }
        disabled={!view?.undoLabel}
        onClick={undo}
      />
      <CbButton
        icon="redo"
        label={
          view?.redoLabel
            ? `Redo ${view.redoLabel} (${shortcutLabel("Mod+Shift+Z")})`
            : "Nothing to redo"
        }
        disabled={!view?.redoLabel}
        onClick={redo}
      />
    </div>
  );
}

function Transport() {
  const [inputOpen,setInputOpen]=useState(false);
  const [midiOpen,setMidiOpen]=useState(false);
  const capture=useCapture();
  const midiRecording=useMidi(s=>!!s.status?.recording);
  const recording=!!capture.status?.active&&!capture.status.monitoring||midiRecording;
  const engine = useApp((s) => s.engine);
  const snapshot = useTransport((s) => s.snapshot);
  const available = engineAvailable(engine);
  const reason = engine?.state === "unavailable" ? engine.reason : "";
  const label = (what: string) => (available ? what : `${what} — unavailable: ${reason}`);
  const rolling = isRolling(snapshot);
  return (
    <div className="cb-transport" role="group" aria-label="Transport">
      <span className="transport-secondary"><CbButton icon="toStart" label="Go to beginning (Home)" onClick={() => locate(0)} /></span>
      <CbButton icon="stop" label={label("Stop")} disabled={!available} onClick={stop} active={snapshot?.state === "stopped" ? undefined : false} />
      {rolling ? (
        <CbButton icon="pause" tone="play" label={label("Pause (Space stops)")} disabled={!available||recording} onClick={pause} active size={17} />
      ) : (
        <CbButton icon="play" tone="play" label={label("Play (Space)")} disabled={!available} onClick={play} size={17} />
      )}
      <CbButton icon="record" tone="record" label={capture.status?.active?(capture.status.monitoring?"Stop input monitoring":"Stop recording"):"Record armed audio tracks"} active={capture.status?.active&&!capture.status.monitoring} disabled={!available||capture.pending||midiRecording} onClick={()=>void(capture.status?.active?capture.stop():capture.start())}/>
      <button type="button" className="cb-btn transport-secondary" aria-label="Audio recording settings" onClick={()=>setInputOpen(true)}>Input</button>
      <RecordingDialog open={inputOpen} onClose={()=>setInputOpen(false)}/>
      <button type="button" className="cb-btn transport-secondary" aria-label="MIDI devices and monitor" onClick={()=>setMidiOpen(true)}>MIDI</button>
      <MidiDialog open={midiOpen} onClose={()=>setMidiOpen(false)}/>
    </div>
  );
}

function Lcd() {
  const cursorTicks = useUi((s) => s.cursorTicks);
  const snapshot = useTransport((s) => s.snapshot);
  const rolling = isRolling(snapshot);
  const cursor = rolling && snapshot ? Math.floor(snapshot.ticks) : cursorTicks;
  const transport = useProject((s) => s.view?.project.transport);
  const [mapOpen, setMapOpen] = useState(false);
  const apply = useProject((s) => s.apply);
  const sigMenu = useMenu();
  const keyMenu = useMenu();
  if (!transport) return <div className="lcd" />;
  const ts = transport.timeSignature;
  const map = TempoMap.fromTransport(transport);
  const bar = map.barAt(cursor);
  const p = positionFromTicks(cursor - bar.start, bar.ts);
  p.bar = bar.bar + 1;
  const seconds = rolling && snapshot ? snapshot.seconds : map.secondsAt(cursor);
  const countIn = snapshot?.state === "countIn" ? snapshot.countInRemaining : 0;
  const bpmNow = map.bpmAt(cursor);

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
      onSelect: () =>
        void apply({
          type: "setTimeSignature",
          numerator: n!,
          denominator: d!,
        }),
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
            onCommit={(numerator) =>
              void apply({
                type: "setTimeSignature",
                numerator,
                denominator: ts.denominator,
              })
            }
          />
          <span>/</span>
          <select
            aria-label="Beat unit"
            value={ts.denominator}
            onChange={(e) =>
              void apply({
                type: "setTimeSignature",
                numerator: ts.numerator,
                denominator: Number(e.target.value),
              })
            }
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

  const setKey = (tonic: PitchClass, mode: KeyMode) =>
    void apply({ type: "setKey", key: { tonic, mode } });
  const keyEntries: MenuEntry[] = [
    {
      key: "keys",
      custom: (
        <div className="key-grid">
          {(["major", "minor"] as const).map((mode) => (
            <div key={mode} className="key-grid__col">
              <div className="menu__heading">
                {mode === "major" ? "Major" : "Minor"}
              </div>
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
      <div
        className="lcd__cell lcd__position"
        title={rolling ? "Playhead position reported by the audio engine" : "Cursor position"}
        data-testid="lcd-position"
      >
        <div className="lcd__clock">{countIn > 0 ? `Count-in ${countIn.toFixed(1)} s` : formatClockSeconds(seconds)}</div>
        <div className="lcd__digits">
          <span className="lcd__big"><span className="lcd__zero">{String(p.bar).padStart(3, "0").slice(0, -String(p.bar).length)}</span>{p.bar}</span>
          <span className="lcd__big">{p.beat}</span>
          <span className="lcd__small">{p.division}</span>
          <span className="lcd__small">{String(p.tick).padStart(3, "0")}</span>
        </div>
        <div className="lcd__position-labels"><span>COMPÁS</span><span>TIEMPO</span></div>
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
          <button type="button" className="lcd__tap" title="Tap tempo (native clock; applies from the third tap)" onClick={() => void tapTempo()}>
            CONSERVAR
          </button>
          <span title="Tempo at the playhead">{transport.tempoChanges.length > 0 ? `ahora ${Number(bpmNow.toFixed(2))}` : "TEMPO"}</span>
        </div>
      </div>
      <div className="lcd__cell lcd__meta">
        <button
          type="button"
          className="lcd__btn"
          title="Time signature"
          onClick={(e) => sigMenu.openBelow(e.currentTarget)}
        >
          {`${bar.ts.numerator}/${bar.ts.denominator}`}
        </button>
        <button
          type="button"
          className="lcd__btn lcd__btn--sub"
          title="Key"
          onClick={(e) => keyMenu.openBelow(e.currentTarget)}
        >
          {`${({ C: "Do", D: "Re", E: "Mi", F: "Fa", G: "Sol", A: "La", B: "Si" } as Record<string, string>)[transport.key.tonic] ?? transport.key.tonic} ${transport.key.mode === "major" ? "may." : "men."}`}
        </button>
        <button
          type="button"
          className="lcd__btn lcd__map"
          title="Tempo, metre, markers, cycle/punch, count-in and metronome"
          onClick={() => setMapOpen(true)}
        >
          <Icon name="chevronDown" size={15} />
        </button>
      </div>
      <TimelineSettingsDialog open={mapOpen} onClose={() => setMapOpen(false)} />
      {sigMenu.at && (
        <Menu at={sigMenu.at} entries={sigEntries} onClose={sigMenu.close} />
      )}
      {keyMenu.at && (
        <Menu at={keyMenu.at} entries={keyEntries} onClose={keyMenu.close} />
      )}
    </div>
  );
}

function Modes() {
  const view = useProject((s) => s.view);
  const follow = useTransport((s) => s.follow);
  const setFollow = useTransport((s) => s.setFollow);
  const t = view?.project.transport;
  const loop = t?.loopRange.enabled ?? false;
  return (
    <div className="cb-group" role="group" aria-label="Modes">
      <CbButton icon="loop" tone="cycle" label={loop ? "Cycle on (L)" : "Cycle off (L)"} active={loop} onClick={toggleLoop} disabled={!view} />
      <CbButton
        icon="metronome"
        tone="cycle"
        label={t?.metronome.enabled ? "Metronome on (K)" : "Metronome off (K)"}
        active={t?.metronome.enabled ?? false}
        onClick={toggleMetronome}
        disabled={!view}
      />
      <button
        type="button"
        className={`cb-btn cb-btn--cycle cb-text${(t?.countInBars ?? 0) > 0 ? " is-active" : ""}`}
        aria-pressed={(t?.countInBars ?? 0) > 0}
        title={(t?.countInBars ?? 0) > 0 ? `Count-in: ${t?.countInBars} bar(s) before playback` : "Count-in off"}
        onClick={toggleCountIn}
        disabled={!view}
      >
        1234
      </button>
      <CbButton icon="forward" tone="view" label={follow ? "Follow playhead on" : "Follow playhead off"} active={follow} onClick={() => setFollow(!follow)} />
    </div>
  );
}

function ProjectTitle() {
  const view = useProject((s) => s.view);
  const apply = useProject((s) => s.apply);
  const [editing, setEditing] = useState(false);
  if (!view) return <div className="project-title" />;
  const where =
    view.path ??
    (view.origin === "demo" ? "Demo session — not saved" : "Not saved yet");
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
        {view.dirty && (
          <span
            className="dirty-dot"
            title="Unsaved changes"
            aria-label="Unsaved changes"
          />
        )}
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
        <button
          type="button"
          className="status__update"
          onClick={() => setUpdateOpen(true)}
          title="An update is available"
        >
          <Icon name="download" size={12} />
          <span>Update</span>
        </button>
      )}
      <div
        className={`status__pill ${engine?.state !== "running" ? "is-offline" : engine.output === "device" ? "is-ok" : "is-warn"}`}
        title={
          engine?.state === "running"
            ? `${engine.device} · ${engine.config.sampleRate} Hz${engine.config.bufferFrames ? ` · ${engine.config.bufferFrames} frames` : ""}`
            : (engine?.reason ?? "Audio engine starting")
        }
        data-testid="engine-status"
      >
        <span className="status__dot" />
        <span className="status__text">
          {engine?.state !== "running" ? "Engine offline" : engine.output === "device" ? `${engine.config.sampleRate / 1000} kHz` : "No audio output"}
        </span>
      </div>
    </div>
  );
}

export function TopBar() {
  const layout = useUi((s) => s.layout);
  const setLayout = useUi((s) => s.setLayout);
  const openSettings = useUi((s) => s.openSettings);
  const chrome = useChrome();
  return (
    <><AppMenu /><div className="document-titlebar"><ProjectTitle /><span> - Pistas</span></div>
    {chrome.controlsVisible && <header className="controlbar">
      <div className="controlbar__side">
        <div className="cb-group" role="group" aria-label="Biblioteca e inspector">
          <CbButton icon="library" tone="view" label="Biblioteca (Y)" active={layout.browserVisible} onClick={() => setLayout({ browserVisible: !layout.browserVisible })} />
          <CbButton icon="inspector" tone="view" label="Inspector (I)" active={layout.inspectorVisible} onClick={() => setLayout({ inspectorVisible: !layout.inspectorVisible })} />
          <CbButton icon="help" label="Ayuda rápida" onClick={() => pendingControl("Ayuda rápida")} />
          <CbButton icon="trayDownload" label="Biblioteca de sonidos" onClick={() => setLayout({ browserVisible: true })} />
        </div>
        <ViewButtons />
      </div>
      <div className="controlbar__center">
        <Transport />
        <Lcd />
        <Modes />
      </div>
      <div className="controlbar__side controlbar__side--end">
        <div className="cb-group" role="group" aria-label="Panels">
          <CbButton icon="list" tone="view" label="Editores de lista (D)" active={chrome.sidePanel === "events"} onClick={() => chrome.toggleSidePanel("events")} />
          <CbButton icon="note" tone="view" label="Bloc de notas" active={chrome.sidePanel === "notes"} onClick={() => chrome.toggleSidePanel("notes")} />
          <CbButton icon="loops" tone="view" label="Explorador de loops (O)" active={chrome.sidePanel === "loops"} onClick={() => chrome.toggleSidePanel("loops")} />
          <CbButton icon="importAudio" tone="view" label="Exploradores (F)" active={chrome.sidePanel === "files"} onClick={() => chrome.toggleSidePanel("files")} />
        </div>
      </div>
      <details className="controlbar__extras"><summary title="Más controles"><Icon name="more" size={16} /></summary><div><FileButtons /><StatusIndicator /><button onClick={() => openSettings()}>Ajustes</button><button onClick={() => chrome.setDialog("recording")}>Ajustes de grabación…</button><button onClick={() => chrome.setDialog("midi")}>Dispositivos MIDI…</button></div></details>
    </header>}
    <RecordingDialog open={chrome.dialog === "recording"} onClose={() => chrome.setDialog(null)} />
    <MidiDialog open={chrome.dialog === "midi"} onClose={() => chrome.setDialog(null)} />
    <TimelineSettingsDialog open={chrome.dialog === "transport"} onClose={() => chrome.setDialog(null)} />
    </>
  );
}
