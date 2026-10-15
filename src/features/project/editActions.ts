// Commands shared by menus, keyboard shortcuts, context menus and buttons.

import type { TrackKind } from "../../bindings/TrackKind";
import { isEditableTarget } from "../../lib/platform";
import { notify } from "../../state/notifications";
import { currentProject, useProject } from "../../state/project";
import { useUi } from "../../state/ui";

export async function addTrack(kind: TrackKind) {
  const ui = useUi.getState();
  const project = currentProject();
  // Insert below the last selected track, like most DAWs.
  const lastSelected = ui.selection.trackIds[ui.selection.trackIds.length - 1];
  const index = lastSelected ? project?.tracks.findIndex((t) => t.id === lastSelected) : -1;
  const outcome = await useProject.getState().apply({
    type: "addTrack",
    kind,
    ...(index !== undefined && index >= 0 ? { index: index + 1 } : {}),
  });
  const id = outcome?.createdTracks[0];
  if (id) ui.selectTracks([id], id);
}

export async function duplicateSelectedTracks() {
  const ids = useUi.getState().selection.trackIds;
  if (!ids.length) return notify("info", "Select one or more tracks to duplicate.");
  const outcome = await useProject.getState().apply({ type: "duplicateTracks", ids });
  if (outcome?.createdTracks.length) useUi.getState().selectTracks(outcome.createdTracks);
}

export async function deleteSelectedTracks() {
  const ids = useUi.getState().selection.trackIds;
  if (!ids.length) return;
  await useProject.getState().apply({ type: "removeTracks", ids });
}

/** Delete key: regions take precedence over tracks. */
export async function deleteSelection() {
  const { regionIds, trackIds } = useUi.getState().selection;
  if (regionIds.length) {
    await useProject.getState().apply({ type: "removeRegions", ids: regionIds });
    useUi.getState().selectRegions([]);
  } else if (trackIds.length) {
    await deleteSelectedTracks();
  }
}

export function setMuted(ids: string[], muted: boolean) {
  return useProject.getState().apply({ type: "setTracksMuted", ids, muted });
}

export function setSoloed(ids: string[], soloed: boolean) {
  return useProject.getState().apply({ type: "setTracksSoloed", ids, soloed });
}

/**
 * Undo/redo from the menu or keyboard. When a text field has focus the
 * shortcut edits the text instead of the project.
 */
export function undo() {
  if (isEditableTarget(document.activeElement)) {
    document.execCommand("undo");
    return;
  }
  void useProject.getState().undo();
}

export function redo() {
  if (isEditableTarget(document.activeElement)) {
    document.execCommand("redo");
    return;
  }
  void useProject.getState().redo();
}

export function toggleLoop() {
  const p = currentProject();
  if (!p) return;
  const { loopRange } = p.transport;
  void useProject.getState().apply({ type: "setLoop", enabled: !loopRange.enabled, start: loopRange.start, end: loopRange.end });
}

export function zoomBy(factor: number) {
  const ui = useUi.getState();
  ui.setZoom(ui.pxPerQuarter * factor);
}

export function engineUnavailableNotice(what: string) {
  notify("info", `${what} needs the audio engine, which is not part of this build yet.`);
}
