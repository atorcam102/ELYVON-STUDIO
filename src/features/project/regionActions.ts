// Region editing and audio import. Every action is a single native edit
// (one undo step); the UI only decides *what* to ask for.

import { open, save } from "@tauri-apps/plugin-dialog";
import type { ClipboardRegion } from "../../bindings/ClipboardRegion";
import type { ImportLayout } from "../../bindings/ImportLayout";
import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import { inTauri, ipc, toIpcError } from "../../lib/ipc";
import { notify, notifyError } from "../../state/notifications";
import { currentProject, enqueue, useProject } from "../../state/project";
import { useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";

/** Formats accepted by the importer (mirrors `media::SUPPORTED_EXTENSIONS`). */
export const AUDIO_EXTENSIONS = ["wav", "wave", "aif", "aiff", "flac", "mp3", "ogg", "oga"];

export function isAudioPath(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return AUDIO_EXTENSIONS.includes(ext);
}

let clipboard: ClipboardRegion[] = [];

export function clipboardSize(): number {
  return clipboard.length;
}

function selectedRegions(project: Project): { region: Region; trackIndex: number }[] {
  const ids = new Set(useUi.getState().selection.regionIds);
  const out: { region: Region; trackIndex: number }[] = [];
  project.tracks.forEach((t, i) => t.regions.forEach((r) => ids.has(r.id) && out.push({ region: r, trackIndex: i })));
  return out;
}

/** The edit position: the engine playhead while rolling, else the cursor. */
function playheadTick(): number {
  const s = useTransport.getState().snapshot;
  return s && (s.state === "playing" || s.state === "countIn") ? Math.round(s.ticks) : useUi.getState().cursorTicks;
}

export async function splitSelectedAtPlayhead() {
  const ids = useUi.getState().selection.regionIds;
  if (!ids.length) return notify("info", "Select the regions to split.");
  const outcome = await useProject.getState().apply({ type: "splitRegions", ids, at: playheadTick() });
  if (outcome) useUi.getState().selectRegions([...ids, ...outcome.createdRegions]);
}

export async function duplicateSelectedRegions() {
  const ids = useUi.getState().selection.regionIds;
  if (!ids.length) return notify("info", "Select the regions to duplicate.");
  const outcome = await useProject.getState().apply({ type: "duplicateRegions", ids });
  if (outcome?.createdRegions.length) useUi.getState().selectRegions(outcome.createdRegions);
}

export async function setSelectedRegionsMuted(muted?: boolean) {
  const project = currentProject();
  if (!project) return;
  const sel = selectedRegions(project);
  if (!sel.length) return notify("info", "Select the regions to mute.");
  const next = muted ?? !sel.every((s) => s.region.muted);
  await useProject.getState().apply({ type: "setRegionsMuted", ids: sel.map((s) => s.region.id), muted: next });
}

/** Copies the selected regions to the Elyvon clipboard. Returns the count. */
export function copySelectedRegions(): number {
  const project = currentProject();
  if (!project) return 0;
  const sel = selectedRegions(project);
  if (!sel.length) return 0;
  const first = Math.min(...sel.map((s) => s.trackIndex));
  clipboard = sel.map((s) => ({ trackOffset: s.trackIndex - first, region: s.region }));
  return sel.length;
}

export async function cutSelectedRegions() {
  const n = copySelectedRegions();
  if (!n) return;
  await useProject.getState().apply({ type: "removeRegions", ids: clipboard.map((c) => c.region.id) });
  useUi.getState().selectRegions([]);
}

/** Pastes at the playhead on the selected (or first matching) track. */
export async function pasteRegions() {
  const project = currentProject();
  if (!project || !clipboard.length) return;
  const ui = useUi.getState();
  const wantAudio = clipboard[0]!.region.content.type === "audio";
  const selectedTrack = project.tracks.find((t) => ui.selection.trackIds.includes(t.id));
  const track =
    selectedTrack && (selectedTrack.kind === "audio") === wantAudio
      ? selectedTrack
      : project.tracks.find((t) => (t.kind === "audio") === wantAudio);
  if (!track) return notify("warning", `Add ${wantAudio ? "an audio" : "an instrument or MIDI"} track to paste these regions.`);
  const outcome = await useProject.getState().apply({ type: "pasteRegions", trackId: track.id, at: playheadTick(), items: clipboard });
  if (outcome?.createdRegions.length) ui.selectRegions(outcome.createdRegions, [track.id]);
}

/** Imports files, reporting files that could not be decoded. */
export async function importAudioFiles(paths: string[], trackId: string | null, start: number, layout: ImportLayout) {
  const audio = paths.filter(isAudioPath);
  if (audio.length < paths.length) {
    notify("warning", `${paths.length - audio.length} file(s) skipped: supported formats are ${AUDIO_EXTENSIONS.join(", ")}.`);
  }
  if (!audio.length) return;
  notify("info", `Importing ${audio.length} file(s)…`);
  try {
    const result = await enqueue(() => ipc.media.import(audio, trackId, Math.max(0, Math.round(start)), layout));
    useProject.getState().setView(result.view);
    for (const f of result.failures) notify("error", `${f.path.split(/[\\/]/).pop()}: ${f.message}`);
    if (result.outcome.createdRegions.length) {
      useUi.getState().selectRegions(result.outcome.createdRegions, result.outcome.createdTracks);
      notify("success", `Imported ${result.outcome.createdRegions.length} region(s). The original files are never modified.`);
    }
  } catch (e) {
    notifyError(toIpcError(e), "Import failed");
  }
}

/** File ▸ Import Audio…: new tracks at the cursor (or onto the selected audio track). */
export async function importAudioDialog() {
  if (!inTauri) return;
  const picked = await open({ multiple: true, title: "Import audio files", filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
  if (!picked) return;
  const paths = Array.isArray(picked) ? picked : [picked];
  const project = currentProject();
  const ui = useUi.getState();
  const audioTrack = project?.tracks.find((t) => ui.selection.trackIds.includes(t.id) && t.kind === "audio");
  await importAudioFiles(paths, audioTrack?.id ?? null, ui.cursorTicks, audioTrack ? "sequential" : "newTracks");
}

export async function relinkMedia(mediaId: string) {
  const picked = await open({ multiple: false, title: "Locate the audio file", filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
  if (!picked || Array.isArray(picked)) return;
  try {
    useProject.getState().setView(await enqueue(() => ipc.media.relink(mediaId, picked)));
    notify("success", "File relinked.");
  } catch (e) {
    notifyError(e, "Relink failed");
  }
}

/** Renders the whole song offline and writes WAV + JSON diagnostic report. */
export async function exportDiagnosticRender() {
  const project = currentProject();
  if (!project) return;
  const path = await save({ title: "Export render (32-bit float WAV)", defaultPath: `${project.name}.wav`, filters: [{ name: "WAV", extensions: ["wav"] }] });
  if (!path) return;
  try {
    notify("info", "Rendering…");
    const r = await ipc.media.render(path, false);
    const db = (v: number) => (v > 0 ? `${(20 * Math.log10(v)).toFixed(1)} dBFS` : "−∞");
    notify(
      r.missingMedia > 0 ? "warning" : "success",
      `Rendered ${r.seconds.toFixed(2)} s at ${r.sampleRate} Hz, ${r.clips} clip(s), peak ${db(Math.max(...r.peak))}` +
        (r.missingMedia > 0 ? `, ${r.missingMedia} region(s) silent (missing files)` : "") +
        `. Report: ${path}.json`,
    );
  } catch (e) {
    notifyError(e, "Render failed");
  }
}
