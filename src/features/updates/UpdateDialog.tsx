import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Icon } from "../../components/Icon";
import { formatBytes } from "../../lib/format";
import { ipc, toIpcError } from "../../lib/ipc";
import { notifyError } from "../../state/notifications";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { useUpdates } from "../../state/updates";
import { resolveUnsaved } from "../project/projectActions";

function Notes({ text }: { text: string | null }) {
  if (!text) return <p className="muted">No release notes were provided.</p>;
  // Release notes are shown as plain text (no HTML from the network).
  return <pre className="release-notes">{text}</pre>;
}

/** Explains and drives the update workflow; every step needs a click. */
export function UpdateDialog() {
  const open = useUi((s) => s.updateOpen);
  const setOpen = useUi((s) => s.setUpdateOpen);
  const { state, check, download, cancel, install } = useUpdates();
  const dirty = useProject((s) => s.view?.dirty ?? false);
  const busy = state.phase === "checking" || state.phase === "downloading" || state.phase === "installing";

  const doInstall = async () => {
    // Never install over unsaved work (Windows closes the app to install).
    const decision = await resolveUnsaved("install the update");
    if (decision === "cancel") return;
    if (decision === "discard") {
      try {
        useProject.getState().setView(await ipc.project.new(true));
      } catch (e) {
        return notifyError(e);
      }
    }
    try {
      await install();
    } catch (e) {
      notifyError(toIpcError(e));
    }
  };

  const restart = async () => {
    const decision = await resolveUnsaved("restart");
    if (decision === "cancel") return;
    try {
      if (decision === "discard") await ipc.project.new(true);
      await ipc.app.restart();
    } catch (e) {
      notifyError(e, "Could not restart");
    }
  };

  let body: React.ReactNode;
  let footer: React.ReactNode = <Button onClick={() => setOpen(false)}>Close</Button>;

  switch (state.phase) {
    case "idle":
      body = <p>Check GitHub Releases for a newer version of Elyvon Studio.</p>;
      footer = (
        <>
          <Button onClick={() => setOpen(false)}>Close</Button>
          <Button variant="primary" onClick={() => void check()}>
            Check now
          </Button>
        </>
      );
      break;
    case "checking":
      body = (
        <p className="with-spinner">
          <span className="spinner" /> Checking for updates…
        </p>
      );
      break;
    case "unavailable":
      body = (
        <div className="callout callout--info">
          <Icon name="info" size={16} />
          <p>{state.reason}</p>
        </div>
      );
      break;
    case "upToDate":
      body = (
        <div className="callout callout--ok">
          <Icon name="check" size={16} />
          <p>Elyvon Studio {state.currentVersion} is the latest version.</p>
        </div>
      );
      footer = (
        <>
          <Button onClick={() => void check()}>Check again</Button>
          <Button variant="primary" onClick={() => setOpen(false)}>
            Done
          </Button>
        </>
      );
      break;
    case "prereleaseSkipped":
      body = (
        <div className="callout callout--info">
          <Icon name="info" size={16} />
          <p>
            You are on the latest stable version ({state.currentVersion}). A pre-release ({state.version}) exists, but stable installations are
            never moved to pre-releases automatically. Pre-releases can be downloaded manually from GitHub Releases.
          </p>
        </div>
      );
      break;
    case "available":
    case "downloading":
    case "ready":
    case "installing":
    case "installed": {
      const info = state.info;
      body = (
        <>
          <div className="version-compare">
            <div>
              <span className="muted">Installed</span>
              <strong>{info.currentVersion}</strong>
            </div>
            <Icon name="chevronRight" size={16} />
            <div>
              <span className="muted">Available</span>
              <strong className="accent">{info.version}</strong>
            </div>
            <div className="version-compare__platform">
              <span className="muted">Package</span>
              <strong>{info.platformLabel}</strong>
            </div>
          </div>
          <h4 className="section-label">What’s new</h4>
          <Notes text={info.notes} />
          {state.phase === "downloading" && (
            <div className="progress-block">
              <div className="progress">
                <div
                  className={`progress__bar${state.total ? "" : " is-indeterminate"}`}
                  style={state.total ? { width: `${Math.min(100, (state.downloaded / state.total) * 100)}%` } : undefined}
                />
              </div>
              <span className="muted">
                {formatBytes(state.downloaded)}
                {state.total ? ` of ${formatBytes(state.total)}` : ""}
              </span>
            </div>
          )}
          {state.phase === "ready" && (
            <div className="callout callout--ok">
              <Icon name="check" size={16} />
              <p>
                Downloaded and signature verified.
                {dirty && " You have unsaved changes — you will be asked to save them before installing."}
              </p>
            </div>
          )}
          {state.phase === "installing" && (
            <p className="with-spinner">
              <span className="spinner" /> Installing…
            </p>
          )}
          {state.phase === "installed" && (
            <div className="callout callout--ok">
              <Icon name="check" size={16} />
              <p>Version {info.version} is installed. Restart Elyvon Studio to start using it.</p>
            </div>
          )}
        </>
      );
      if (state.phase === "available") {
        footer = (
          <>
            <Button onClick={() => setOpen(false)}>Later</Button>
            <Button variant="primary" icon="download" onClick={() => void download()}>
              Download
            </Button>
          </>
        );
      } else if (state.phase === "downloading") {
        footer = <Button onClick={() => void cancel()}>Cancel download</Button>;
      } else if (state.phase === "ready") {
        footer = (
          <>
            <Button onClick={() => setOpen(false)}>Later</Button>
            <Button variant="primary" onClick={() => void doInstall()}>
              Install update
            </Button>
          </>
        );
      } else if (state.phase === "installed") {
        footer = (
          <>
            <Button onClick={() => setOpen(false)}>Restart later</Button>
            <Button variant="primary" onClick={() => void restart()}>
              Restart now
            </Button>
          </>
        );
      } else {
        footer = null;
      }
      break;
    }
    case "error":
      body = (
        <div className="callout callout--error">
          <Icon name="warning" size={16} />
          <p>{state.error.message}</p>
        </div>
      );
      footer = (
        <>
          <Button onClick={() => setOpen(false)}>Close</Button>
          <Button variant="primary" onClick={() => void (state.info && state.error.code !== "noRelease" ? download() : check())}>
            Try again
          </Button>
        </>
      );
      break;
  }

  return (
    <Dialog open={open} title="Software Update" onClose={() => setOpen(false)} locked={state.phase === "installing"} width={520} footer={footer}>
      <div className={busy ? "is-busy" : undefined}>{body}</div>
    </Dialog>
  );
}
