import { useEffect, useState } from "react";
import { Splitter } from "./components/Splitter";
import { Toasts } from "./components/Toasts";
import { useAppEvents } from "./features/app/useAppEvents";
import { Library } from "./features/browser/Library";
import { SidePanel } from "./features/browser/SidePanel";
import {
  AboutDialog,
  PromptHost,
  RecoveryDialog,
} from "./features/dialogs/Dialogs";
import { BottomPanel } from "./features/editors/BottomPanel";
import { Inspector } from "./features/inspector/Inspector";
import { SettingsDialog } from "./features/settings/SettingsDialog";
import { MediaDialog } from "./features/dialogs/MediaDialog";
import { Timeline } from "./features/timeline/Timeline";
import { TopBar } from "./features/topbar/TopBar";
import { UpdateDialog } from "./features/updates/UpdateDialog";
import { inTauri, ipc, toIpcError } from "./lib/ipc";
import { useApp } from "./state/app";
import { useProject } from "./state/project";
import { useUi } from "./state/ui";

function Workspace() {
  const layout = useUi((s) => s.layout);
  const setLayout = useUi((s) => s.setLayout);
  const [viewportHeight, setViewportHeight] = useState(window.innerHeight);
  useEffect(() => {
    const resize = () => setViewportHeight(window.innerHeight);
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, []);
  const bottomMax = Math.max(180, viewportHeight - 180);
  const bottomHeight = Math.max(180, Math.min(bottomMax, layout.bottomHeight));
  return (
    <div className="workspace">
      <div className="workspace__main">
        {layout.browserVisible && (
          <>
            <div
              className="workspace__browser"
              style={{ width: layout.browserWidth }}
            >
              <Library />
            </div>
            <Splitter
              orientation="vertical"
              label="Resize browser"
              value={layout.browserWidth}
              min={180}
              max={560}
              direction={1}
              onChange={(browserWidth) => setLayout({ browserWidth })}
            />
          </>
        )}
        {layout.inspectorVisible && (
          <>
            <div
              className="workspace__inspector"
              style={{ width: layout.inspectorWidth }}
            >
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
        <SidePanel />
      </div>
      {layout.bottomVisible && (
        <>
          <Splitter
            orientation="horizontal"
            label="Resize editors"
            value={bottomHeight}
            min={180}
            max={bottomMax}
            direction={-1}
            onChange={(bottomHeight) => setLayout({ bottomHeight })}
            onCollapse={() => setLayout({ bottomVisible: false })}
          />
          <div
            className="workspace__bottom"
            style={{ height: bottomHeight }}
          >
            <BottomPanel />
          </div>
        </>
      )}
    </div>
  );
}

function StartupError({ message }: { message: string }) {
  return (
    <div className="startup-error">
      <h1>Elyvon Studio could not start</h1>
      <p>{message}</p>
      {!inTauri && (
        <p className="muted">
          This interface must run inside the Elyvon Studio desktop application
          (npm run tauri dev).
        </p>
      )}
    </div>
  );
}

export default function App() {
  const [error, setError] = useState<string | null>(null);
  const ready = useProject((s) => s.view !== null);
  useAppEvents();

  useEffect(() => {
    if (import.meta.env.DEV && !inTauri && new URLSearchParams(location.search).has("design-preview")) {
      void import("./reference/designPreview").then(m => m.initializeDesignPreview());
      return;
    }
    Promise.all([
      useApp.getState().load(),
      ipc.project.get().then((v) => useProject.getState().setView(v)),
    ]).catch((e) => setError(toIpcError(e).message));
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
      <MediaDialog />
      <PromptHost />
      <Toasts />
    </div>
  );
}
