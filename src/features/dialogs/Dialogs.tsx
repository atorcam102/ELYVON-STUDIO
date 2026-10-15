import { useEffect, useState } from "react";
import type { RecoveryInfo } from "../../bindings/RecoveryInfo";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { ipc } from "../../lib/ipc";
import { useApp } from "../../state/app";
import { useDialogs } from "../../state/dialogs";
import { notifyError } from "../../state/notifications";
import { enqueue, useProject } from "../../state/project";
import { useUi } from "../../state/ui";

/** Renders prompts requested through `ask()`. */
export function PromptHost() {
  const prompt = useDialogs((s) => s.prompt);
  const close = useDialogs((s) => s.close);
  if (!prompt) return null;
  return (
    <Dialog
      open
      title={prompt.title}
      onClose={() => close(prompt.cancelId)}
      width={440}
      footer={prompt.buttons.map((b) => (
        <Button key={b.id} variant={b.kind ?? "default"} onClick={() => close(b.id)}>
          {b.label}
        </Button>
      ))}
    >
      <p className="prompt__message">{prompt.message}</p>
      {prompt.detail && <p className="muted">{prompt.detail}</p>}
    </Dialog>
  );
}

export function AboutDialog() {
  const open = useUi((s) => s.aboutOpen);
  const setOpen = useUi((s) => s.setAboutOpen);
  const info = useApp((s) => s.info);
  return (
    <Dialog open={open} title="About Elyvon Studio" onClose={() => setOpen(false)} width={440} footer={<Button variant="primary" onClick={() => setOpen(false)}>Close</Button>}>
      <div className="about">
        <img src="/elyvon-studio-icon.svg" alt="" width={64} height={64} />
        <div>
          <h3>Elyvon Studio</h3>
          <p className="muted">
            Version {info?.version ?? "—"}
            {info?.debugBuild ? " (development build)" : ""}
          </p>
          <p className="muted">{info?.rustTarget ?? `${info?.os ?? ""} ${info?.arch ?? ""}`}</p>
        </div>
      </div>
      <p>The first application of the Elyvon creative suite. This early release establishes the project format, interface and update infrastructure; the audio engine follows in Phase 2.</p>
      <dl className="facts">
        <dt>Logs</dt>
        <dd className="selectable">{info?.logDir}</dd>
        <dt>Preferences</dt>
        <dd className="selectable">{info?.configDir}</dd>
      </dl>
      <p className="muted">© 2026 Elyvon. All rights reserved.</p>
    </Dialog>
  );
}

/** Offers to restore a crash-recovery snapshot found at start-up. */
export function RecoveryDialog() {
  const [info, setInfo] = useState<RecoveryInfo | null>(null);

  useEffect(() => {
    ipc.recovery.check().then(setInfo, () => setInfo(null));
  }, []);

  if (!info) return null;
  const when = info.savedAtUnix ? new Date(info.savedAtUnix * 1000).toLocaleString() : "unknown time";
  const restore = async () => {
    try {
      useProject.getState().setView(await enqueue(() => ipc.recovery.restore()));
    } catch (e) {
      notifyError(e, "Could not restore");
    }
    setInfo(null);
  };
  const discard = async () => {
    await ipc.recovery.discard().catch(() => undefined);
    setInfo(null);
  };
  return (
    <Dialog
      open
      title="Recover unsaved work?"
      onClose={() => void discard()}
      width={460}
      footer={
        <>
          <Button variant="danger" onClick={() => void discard()}>
            Discard
          </Button>
          <Button variant="primary" onClick={() => void restore()}>
            Restore
          </Button>
        </>
      }
    >
      <p>
        Elyvon Studio did not shut down normally. Unsaved changes to <strong>“{info.projectName}”</strong> from {when} were recovered.
      </p>
      {info.originalPath && <p className="muted selectable">{info.originalPath}</p>}
      <p className="muted">Restored work opens as an unsaved project. Save it to keep it.</p>
    </Dialog>
  );
}
