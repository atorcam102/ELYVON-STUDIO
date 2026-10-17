// Global wiring: native menu events, keyboard shortcuts, window-close guard,
// crash-recovery snapshots and the optional start-up update check.

import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { hasPrimaryModifier, isEditableTarget, isWindows } from "../../lib/platform";
import { ipc } from "../../lib/ipc";
import { useApp } from "../../state/app";
import { useDialogs } from "../../state/dialogs";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { useUpdates } from "../../state/updates";
import {
  addTrack,
  deleteSelectedTracks,
  deleteSelection,
  duplicateSelectedTracks,
  redo,
  toggleLoop,
  undo,
  zoomBy,
} from "../project/editActions";
import { newProject, openDemoProject, openProject, quitApp, saveProject, saveProjectAs } from "../project/projectActions";
import {
  copySelectedRegions,
  cutSelectedRegions,
  duplicateSelectedRegions,
  exportDiagnosticRender,
  importAudioDialog,
  pasteRegions,
  setSelectedRegionsMuted,
  splitSelectedAtPlayhead,
} from "../project/regionActions";
import { locate, toggleMetronome, togglePlay, stop } from "../transport/transportActions";
import { connectMidi } from "../../state/midi";
import type { SessionView } from "../../bindings/SessionView";
import { glueSelected } from "../project/editingActions";
import { connectCapture } from "../../state/capture";
import { connectTransport } from "../../state/transport";

const RECOVERY_DELAY_MS = 15_000;
const STARTUP_UPDATE_DELAY_MS = 5_000;

function toggleLayout(key: "inspectorVisible" | "browserVisible" | "bottomVisible") {
  const ui = useUi.getState();
  ui.setLayout({ [key]: !ui.layout[key] });
}

function showMixer() {
  const ui = useUi.getState();
  const showing = ui.layout.bottomVisible && ui.layout.bottomTab === "mixer";
  ui.setLayout(showing ? { bottomVisible: false } : { bottomVisible: true, bottomTab: "mixer" });
}

export function runMenuAction(id: string) {
  const ui = useUi.getState();
  switch (id) {
    case "file.new":
      return void newProject();
    case "file.open":
      return void openProject();
    case "file.openDemo":
      return void openDemoProject();
    case "file.save":
      return void saveProject();
    case "file.saveAs":
      return void saveProjectAs();
    case "file.importAudio":
      return void importAudioDialog();
    case "file.exportRender":
      return void exportDiagnosticRender();
    case "region.split":
      return void splitSelectedAtPlayhead();
    case "region.duplicate":
      return void duplicateSelectedRegions();
    case "region.mute":
      return void setSelectedRegionsMuted();
    case "transport.play":
      return togglePlay();
    case "transport.stop":
      return stop();
    case "transport.toStart":
      return locate(0);
    case "transport.metronome":
      return toggleMetronome();
    case "transport.addMarker":
      return void useProject.getState().apply({ type: "addMarker", tick: useUi.getState().cursorTicks });
    case "edit.undo":
      return undo();
    case "edit.redo":
      return redo();
    case "app.settings":
      return ui.openSettings();
    case "app.about":
      return ui.setAboutOpen(true);
    case "app.checkUpdates":
      ui.setUpdateOpen(true);
      return void useUpdates.getState().check();
    case "app.quit":
      return void quitApp();
    case "track.newAudio":
      return void addTrack("audio");
    case "track.newInstrument":
      return void addTrack("instrument");
    case "track.newMidi":
      return void addTrack("midi");
    case "track.duplicate":
      return void duplicateSelectedTracks();
    case "track.delete":
      return void deleteSelectedTracks();
    case "view.inspector":
      return toggleLayout("inspectorVisible");
    case "view.browser":
      return toggleLayout("browserVisible");
    case "view.bottom":
      return toggleLayout("bottomVisible");
    case "view.mixer":
      return showMixer();
    case "view.zoomIn":
      return zoomBy(1.25);
    case "view.zoomOut":
      return zoomBy(1 / 1.25);
  }
}

function moveTrackSelection(delta: number) {
  const project = useProject.getState().view?.project;
  const ui = useUi.getState();
  if (!project?.tracks.length) return;
  const current = ui.selection.trackIds[ui.selection.trackIds.length - 1];
  const i = current ? project.tracks.findIndex((t) => t.id === current) : -1;
  const next = project.tracks[Math.min(project.tracks.length - 1, Math.max(0, i + delta))];
  if (next) ui.selectTracks([next.id], next.id);
}

/** Single-key and secondary shortcuts handled by the UI (menus own the rest). */
function onKeyDown(e: KeyboardEvent) {
  if (e.defaultPrevented || useDialogs.getState().prompt) return;
  if (document.querySelector(".dialog-backdrop")) return;
  const mod = hasPrimaryModifier(e);

  // Windows convention: Ctrl+Y also redoes.
  if (isWindows && mod && e.key.toLowerCase() === "y") {
    e.preventDefault();
    return redo();
  }
  if (!isEditableTarget(e.target) && mod && !e.shiftKey && e.key.toLowerCase() === "j") {
    e.preventDefault();
    return void glueSelected();
  }
  if (isEditableTarget(e.target) || mod || e.altKey) return;

  switch (e.key) {
    case "Delete":
    case "Backspace":
      e.preventDefault();
      return void deleteSelection();
    case "i":
    case "I":
      return toggleLayout("inspectorVisible");
    case "b":
    case "B":
      return toggleLayout("browserVisible");
    case "e":
    case "E":
      return toggleLayout("bottomVisible");
    case "x":
    case "X":
      return showMixer();
    case "l":
    case "L":
      return toggleLoop();
    case " ":
      e.preventDefault();
      return togglePlay();
    case "Home":
    case "Enter":
      return locate(0);
    case "k":
    case "K":
      return toggleMetronome();
    case "m":
    case "M":
      if (useUi.getState().selection.regionIds.length) return void setSelectedRegionsMuted();
      return;
    case "Escape":
      return useUi.getState().clearSelection();
    case "ArrowUp":
      e.preventDefault();
      return moveTrackSelection(-1);
    case "ArrowDown":
      e.preventDefault();
      return moveTrackSelection(1);
  }
}

export function useAppEvents() {
  // Native menu → UI actions; window close guard.
  useEffect(() => {
    const unlisten = [
      listen<string>("menu", (e) => runMenuAction(e.payload)),
      listen("app://close-requested", () => void quitApp()),
      // Automation written during playback becomes one edit in the core.
      listen<SessionView>("project://view", (e) => useProject.getState().setView(e.payload)),
    ];
    return () => {
      unlisten.forEach((p) => void p.then((f) => f()));
    };
  }, []);

  useEffect(() => {
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // Engine events: playhead, meters, engine and media status.
  useEffect(() => connectTransport(), []);
  useEffect(() => connectCapture(), []);
  useEffect(() => connectMidi(), []);

  // Cut/copy/paste of regions through the native Edit menu or the keyboard.
  // Text fields keep their normal clipboard behaviour.
  useEffect(() => {
    const handler = (kind: "copy" | "cut" | "paste") => (e: ClipboardEvent) => {
      if (isEditableTarget(document.activeElement) || document.querySelector(".dialog-backdrop")) return;
      if (kind === "paste") {
        e.preventDefault();
        return void pasteRegions();
      }
      if (!useUi.getState().selection.regionIds.length) return;
      e.preventDefault();
      if (kind === "copy") copySelectedRegions();
      else void cutSelectedRegions();
    };
    const listeners = (["copy", "cut", "paste"] as const).map((k) => [k, handler(k)] as const);
    listeners.forEach(([k, f]) => document.addEventListener(k, f));
    return () => listeners.forEach(([k, f]) => document.removeEventListener(k, f));
  }, []);

  // Keep selection consistent with the project after every change.
  useEffect(
    () =>
      useProject.subscribe((s, prev) => {
        if (s.view && s.view.project !== prev.view?.project) useUi.getState().pruneSelection(s.view.project);
      }),
    [],
  );

  // Crash recovery: snapshot unsaved changes after a quiet period.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const unsub = useProject.subscribe((s, prev) => {
      if (!s.view?.dirty || s.view.revision === prev.view?.revision) return;
      clearTimeout(timer);
      timer = setTimeout(() => void ipc.recovery.snapshot().catch(() => undefined), RECOVERY_DELAY_MS);
    });
    return () => {
      clearTimeout(timer);
      unsub();
    };
  }, []);

  // One optional background update check, never repeated periodically.
  const settingsLoaded = useApp((s) => s.settings !== null);
  useEffect(() => {
    if (!settingsLoaded) return;
    const { settings, info } = useApp.getState();
    if (!settings?.updates.checkOnStartup || info?.updater.unavailableReason) return;
    const t = setTimeout(() => void useUpdates.getState().check({ background: true }), STARTUP_UPDATE_DELAY_MS);
    return () => clearTimeout(t);
  }, [settingsLoaded]);
}
