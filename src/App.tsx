import { useEffect, useState } from "react";
import { IconButton } from "./components/Button";
import { Splitter } from "./components/Splitter";
import { Toasts } from "./components/Toasts";
import { useAppEvents } from "./features/app/useAppEvents";
import { Browser } from "./features/browser/Browser";
import { AboutDialog, PromptHost, RecoveryDialog } from "./features/dialogs/Dialogs";
import { BottomPanel } from "./features/editors/BottomPanel";
import { Inspector } from "./features/inspector/Inspector";
import { SettingsDialog } from "./features/settings/SettingsDialog";
import { Timeline } from "./features/timeline/Timeline";
import { TopBar } from "./features/topbar/TopBar";
import { UpdateDialog } from "./features/updates/UpdateDialog";
import { inTauri, ipc, toIpcError } from "./lib/ipc";
import { useApp } from "./state/app";
import { useProject } from "./state/project";
import { useUi } from "./state/ui";

function PanelToggles() {
  const layout = useUi((s) => s.layout);
  const setLayout = useUi((s) => s.setLayout);
  return (
    <div className="panel-toggles">
      <IconButton icon="panelLeft" label="Inspector (I)" active={layout.inspectorVisible} size={14} onClick={() => setLayout({ inspectorVisible: !layout.inspectorVisible })} />
      <IconButton icon="panelBottom" label="Editors (E)" active={layout.bottomVisible} size={14} onClick={() => setLayout({ bottomVisible: !layout.bottomVisible })} />
      <IconButton icon="panelRight" label="Browser (B)" active={layout.browserVisible} size={14} onClick={() => setLayout({ browserVisible: !layout.browserVisible })} />
    </div>
  );
}

function Workspace() {
  const layout = useUi((s) => s.layout);
  const setLayout = useUi((s) => s.setLayout);
  return (
    <div className="workspace">
      <div className="workspace__main">
        {layout.inspectorVisible && (
          <>
            <div className="workspace__inspector" style={{ width: layout.inspectorWidth }}>
              <Inspector />
            </div>
            <Splitter
              orientation="vertical"
              label="Resize inspector"
              value={layout.inspectorWidth}
              min={180}
              max={480}
              direction={1}
              onChange={(inspectorWidth) => setLayout({ inspectorWidth })}
            />
          </>
        )}
        <div className="workspace__center">
          <Timeline />
        </div>
        {layout.browserVisible && (
          <>
            <Splitter
              orientation="vertical"
              label="Resize browser"
              value={layout.browserWidth}
              min={180}
              max={560}
              direction={-1}
              onChange={(browserWidth) => setLayout({ browserWidth })}
            />
            <div className="workspace__browser" style={{ width: layout.browserWidth }}>
              <Browser />
            </div>
          </>
        )}
      </div>
      {layout.bottomVisible && (
        <>
          <Splitter
            orientation="horizontal"
            label="Resize editors"
            value={layout.bottomHeight}
            min={140}
            max={720}
            direction={-1}
            onChange={(bottomHeight) => setLayout({ bottomHeight })}
          />
          <div className="workspace__bottom" style={{ height: layout.bottomHeight }}>
            <BottomPanel />
          </div>
        </>
      )}
      <PanelToggles />
    </div>
  );
}

function StartupError({ message }: { message: string }) {
  return (
    <div className="startup-error">
      <h1>Elyvon Studio could not start</h1>
      <p>{message}</p>
      {!inTauri && <p className="muted">This interface must run inside the Elyvon Studio desktop application (npm run tauri dev).</p>}
    </div>
  );
}

export default function App() {
  const [error, setError] = useState<string | null>(null);
  const ready = useProject((s) => s.view !== null);
  useAppEvents();

  useEffect(() => {
    Promise.all([useApp.getState().load(), ipc.project.get().then((v) => useProject.getState().setView(v))]).catch((e) =>
      setError(toIpcError(e).message),
    );
  }, []);

  if (error) return <StartupError message={error} />;
  if (!ready) return <div className="app app--loading" aria-busy="true" />;

  return (
    <div className="app">
      <TopBar />
      <Workspace />
      <SettingsDialog />
      <UpdateDialog />
      <AboutDialog />
      <RecoveryDialog />
      <PromptHost />
      <Toasts />
    </div>
  );
}
