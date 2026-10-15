// User-level project workflows: New, Open, Save, Save As, Quit.
//
// The native side refuses to discard unsaved changes unless told to, so every
// flow first resolves unsaved work with the user (Save / Don't Save / Cancel).

import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { SessionView } from "../../bindings/SessionView";
import { stripProjectExtension } from "../../lib/format";
import { ipc, toIpcError } from "../../lib/ipc";
import { ask } from "../../state/dialogs";
import { notify, notifyError } from "../../state/notifications";
import { enqueue, useProject } from "../../state/project";
import { useUi } from "../../state/ui";

const FILTERS = [{ name: "Elyvon Studio Project", extensions: ["elyvon"] }];

function adopt(view: SessionView) {
  useProject.getState().setView(view);
  useUi.getState().clearSelection();
  useUi.getState().setCursor(0);
}

function view(): SessionView | null {
  return useProject.getState().view;
}

/** Asks for a file name and saves there. Returns false if cancelled or failed. */
export async function saveProjectAs(): Promise<boolean> {
  const v = view();
  if (!v) return false;
  const suggested = v.path ?? `${stripProjectExtension(v.project.name)}.elyvon`;
  const path = await saveDialog({ title: "Save Project As", defaultPath: suggested, filters: FILTERS });
  if (!path) return false;
  return doSave(path);
}

async function doSave(path?: string): Promise<boolean> {
  try {
    adoptSaved(await enqueue(() => ipc.project.save(path)));
    notify("success", "Project saved");
    return true;
  } catch (e) {
    const err = toIpcError(e);
    if (err.code === "needsSaveAs") return saveProjectAs();
    notifyError(err, "Could not save");
    return false;
  }
}

function adoptSaved(v: SessionView) {
  useProject.getState().setView(v);
}

export async function saveProject(): Promise<boolean> {
  const v = view();
  if (!v) return false;
  const needsName = v.path === null || v.origin === "demo" || v.origin === "new";
  return needsName ? saveProjectAs() : doSave();
}

/**
 * Resolves unsaved changes before a destructive action.
 * Returns `"proceed"` (nothing to lose, or saved), `"discard"` or `"cancel"`.
 */
export async function resolveUnsaved(action: string): Promise<"proceed" | "discard" | "cancel"> {
  const v = view();
  if (!v?.dirty) return "proceed";
  const choice = await ask({
    title: "Unsaved changes",
    message: `Do you want to save the changes to “${v.project.name}” before you ${action}?`,
    detail: "Your changes will be lost if you don't save them.",
    buttons: [
      { id: "discard", label: "Don't Save", kind: "danger" },
      { id: "cancel", label: "Cancel" },
      { id: "save", label: "Save", kind: "primary" },
    ],
    cancelId: "cancel",
  });
  if (choice === "save") return (await saveProject()) ? "proceed" : "cancel";
  return choice;
}

async function replaceProject(action: string, load: (discard: boolean) => Promise<SessionView>) {
  const decision = await resolveUnsaved(action);
  if (decision === "cancel") return;
  try {
    adopt(await enqueue(() => load(decision === "discard")));
  } catch (e) {
    const err = toIpcError(e);
    if (err.code === "unsavedChanges") {
      // Edited while the prompt was open; ask again.
      return replaceProject(action, load);
    }
    notifyError(err, "Could not open the project");
  }
}

export function newProject() {
  return replaceProject("create a new project", (discard) => ipc.project.new(discard));
}

export function openDemoProject() {
  return replaceProject("open the demo session", (discard) => ipc.project.openDemo(discard));
}

export async function openProject(path?: string) {
  let target = path;
  if (!target) {
    const picked = await openDialog({ title: "Open Project", multiple: false, directory: false, filters: FILTERS });
    if (typeof picked !== "string") return;
    target = picked;
  }
  const chosen = target;
  return replaceProject("open another project", (discard) => ipc.project.open(chosen, discard));
}

export async function quitApp() {
  const decision = await resolveUnsaved("quit");
  if (decision === "cancel") return;
  try {
    await ipc.app.quit(decision === "discard");
  } catch (e) {
    notifyError(e, "Could not quit");
  }
}
