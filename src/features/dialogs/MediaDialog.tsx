// Project media (Phase 11): file status, repair of missing files, collect
// media for a portable project folder and backed-up clean-up. Source files
// are never modified; the only destructive step (moving unused project files)
// needs an explicit confirmation and keeps a backup.

import { open as chooseOpen } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { inTauri, ipc } from "../../lib/ipc";
import { notify, notifyError } from "../../state/notifications";
import { enqueue, useProject } from "../../state/project";
import { useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";
import { relinkMedia } from "../project/regionActions";

export function MediaDialog() {
  const open = useUi((s) => s.mediaOpen);
  const close = () => {
    setConfirming(false);
    useUi.getState().setMediaOpen(false);
  };
  const view = useProject((s) => s.view);
  const media = useTransport((s) => s.media);
  const [unused, setUnused] = useState<string[] | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!open) return;
    void ipc.repair
      .unused()
      .then(setUnused)
      .catch(() => setUnused(null));
  }, [open, view?.revision]);
  if (!view) return null;
  const project = view.project;
  const used = new Set(
    project.tracks.flatMap((t) => [
      ...t.regions.flatMap((r) => (r.content.type === "audio" && r.content.media ? [r.content.media] : [])),
      ...(t.freeze ? [t.freeze.media] : []),
    ]),
  );
  const failed = project.media.filter((m) => media[m.id]?.state === "failed");
  const run = async (task: () => Promise<void>) => {
    setBusy(true);
    try {
      await task();
    } catch (e) {
      notifyError(e);
    } finally {
      setBusy(false);
    }
  };
  const search = () =>
    run(async () => {
      if (!inTauri) return;
      const dir = await chooseOpen({ directory: true, title: "Search this folder for the missing files" });
      if (typeof dir !== "string") return;
      const r = await enqueue(() => ipc.repair.search(dir));
      useProject.getState().setView(r.view);
      notify(
        r.report.relinked.length ? "success" : "info",
        `Relinked ${r.report.relinked.length} file(s) after checking size and content` +
          (r.report.notFound.length ? `; ${r.report.notFound.length} not found` : "") +
          (r.report.ambiguous.length ? `; ${r.report.ambiguous.length} ambiguous` : "") +
          ` (${r.report.scanned} files scanned${r.report.truncated ? ", search stopped at its limit" : ""}).`,
      );
    });
  const collect = () =>
    run(async () => {
      const v = await enqueue(() => ipc.repair.collect());
      useProject.getState().setView(v);
      notify("success", "Media copied into the project's Media folder; save the project to keep the new links. The originals were not touched.");
    });
  const backup = () =>
    run(async () => {
      const dir = await ipc.repair.backupUnused(true);
      setConfirming(false);
      notify("success", `Moved the unused files to ${dir}. Nothing was deleted.`);
      setUnused(await ipc.repair.unused());
    });

  return (
    <Dialog open={open} title="Project media" onClose={close} width={620} footer={<Button onClick={close}>Close</Button>}>
      <p>
        Elyvon never modifies your audio files: edits change how regions play them. {project.media.length} file(s) in the pool, {failed.length} unavailable.
      </p>
      <div className="media-list" role="list">
        {project.media.map((m) => {
          const s = media[m.id];
          const state =
            s?.state === "ready" ? (s.changed ? "Changed since import" : "OK") : s?.state === "failed" ? (s.code === "missing" ? "Missing" : s.message) : "Loading";
          return (
            <div key={m.id} className={`media-list__row${s?.state === "failed" ? " is-failed" : ""}`} role="listitem">
              <span className="media-list__name" title={m.path}>
                {m.name}
              </span>
              <span className="media-list__state">{used.has(m.id) ? state : `${state} · unused`}</span>
              {s?.state === "failed" && (
                <Button size="sm" disabled={busy} onClick={() => void relinkMedia(m.id)}>
                  Locate…
                </Button>
              )}
            </div>
          );
        })}
      </div>
      <div className="insp-buttons">
        <Button disabled={busy || !failed.length} onClick={() => void search()}>
          Search folder for missing files…
        </Button>
        <Button
          disabled={busy}
          onClick={() =>
            void run(async () => {
              await ipc.repair.rescan();
              notify("info", "Rescanning unavailable files.");
            })
          }
        >
          Rescan
        </Button>
      </div>
      <h3>Portability</h3>
      <p>Collect media copies every file the project uses into a Media folder next to the project file (verified copies, never overwriting) and links to the copies.</p>
      <div className="insp-buttons">
        <Button disabled={busy || !view.path} title={view.path ? undefined : "Save the project first"} onClick={() => void collect()}>
          Collect media into project folder
        </Button>
        <Button disabled={busy || project.media.every((m) => used.has(m.id))} onClick={() => void useProject.getState().apply({ type: "removeUnusedMedia" })}>
          Remove unused from pool
        </Button>
      </div>
      <h3>Clean up</h3>
      {unused === null || unused.length === 0 ? (
        <p>No unused files in the project's Media, Bounces or Freeze folders.</p>
      ) : (
        <>
          <p>{unused.length} file(s) in the project folder are not used by this project:</p>
          <ul className="media-unused">
            {unused.slice(0, 50).map((u) => (
              <li key={u}>{u}</li>
            ))}
          </ul>
          {confirming ? (
            <div className="insp-buttons" role="alert">
              <span>Move these {unused.length} file(s) into “.elyvon-backup” in the project folder?</span>
              <Button variant="danger" disabled={busy} onClick={() => void backup()}>
                Move to backup
              </Button>
              <Button disabled={busy} onClick={() => setConfirming(false)}>
                Cancel
              </Button>
            </div>
          ) : (
            <Button disabled={busy} onClick={() => setConfirming(true)}>
              Move unused files to backup…
            </Button>
          )}
        </>
      )}
    </Dialog>
  );
}
