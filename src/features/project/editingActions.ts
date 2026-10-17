// Non-destructive editing actions (Phase 11). Each is one native edit (one
// undo step); analysis and rendering run in the core.

import type { FadeCurve } from "../../bindings/FadeCurve";
import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import { ipc, toIpcError } from "../../lib/ipc";
import { notify, notifyError } from "../../state/notifications";
import { currentProject, enqueue, useProject } from "../../state/project";
import { useUi } from "../../state/ui";

function selected(project: Project): Region[] {
  const ids = new Set(useUi.getState().selection.regionIds);
  return project.tracks.flatMap((t) => t.regions.filter((r) => ids.has(r.id)));
}

function audioSelection(): Region[] | null {
  const project = currentProject();
  if (!project) return null;
  const regions = selected(project).filter((r) => r.content.type === "audio");
  if (!regions.length) {
    notify("info", "Select one or more audio regions.");
    return null;
  }
  return regions;
}

function trackOf(project: Project, regionId: string) {
  return project.tracks.find((t) => t.regions.some((r) => r.id === regionId));
}

/** Delete with ripple when the ripple mode is on, or the marquee range. */
export async function deleteRegionsOrRange() {
  const ui = useUi.getState();
  if (ui.marquee) {
    const m = ui.marquee;
    await useProject.getState().apply({ type: "cutRange", trackIds: m.trackIds, start: m.start, end: m.end, ripple: ui.ripple });
    ui.setMarquee(null);
    return true;
  }
  if (ui.ripple && ui.selection.regionIds.length) {
    await useProject.getState().apply({ type: "rippleDelete", ids: ui.selection.regionIds });
    ui.selectRegions([]);
    return true;
  }
  return false;
}

export async function rippleDeleteSelected() {
  const ids = useUi.getState().selection.regionIds;
  if (!ids.length) return notify("info", "Select the regions to delete.");
  await useProject.getState().apply({ type: "rippleDelete", ids });
  useUi.getState().selectRegions([]);
}

export async function glueSelected() {
  const ids = useUi.getState().selection.regionIds;
  if (ids.length < 2) return notify("info", "Select two or more regions of one track to glue.");
  const out = await useProject.getState().apply({ type: "glueRegions", ids });
  if (out?.selectedRegions.length) useUi.getState().selectRegions(out.selectedRegions);
}

export async function splitMarquee() {
  const ui = useUi.getState();
  const m = ui.marquee;
  if (!m) return;
  const out = await useProject.getState().apply({ type: "splitRange", trackIds: m.trackIds, start: m.start, end: m.end });
  ui.setMarquee(null);
  if (out) ui.selectRegions(out.selectedRegions, m.trackIds);
}

export async function reverseSelected() {
  const regions = audioSelection();
  if (!regions) return;
  const on = !regions.every((r) => r.content.type === "audio" && r.content.reversed);
  await useProject.getState().apply({ type: "setRegionsReversed", ids: regions.map((r) => r.id), reversed: on });
}

export async function setFadeCurve(curve: FadeCurve) {
  const regions = audioSelection();
  if (!regions) return;
  await useProject.getState().apply({ type: "setFadeCurve", ids: regions.map((r) => r.id), curve });
}

export async function normalizeSelected(targetDb = -1) {
  const regions = audioSelection();
  if (!regions) return;
  try {
    const r = await enqueue(() => ipc.editing.normalize(regions.map((x) => x.id), targetDb));
    useProject.getState().setView(r.view);
    notify("success", `Normalised ${regions.length} region(s) to ${targetDb} dBFS (region gain; the files are unchanged).`);
  } catch (e) {
    notifyError(toIpcError(e), "Normalize failed");
  }
}

/** Crossfades the selected region with the next touching region on its track. */
export async function crossfadeWithNext(lengthMs = 20) {
  const project = currentProject();
  const regions = audioSelection();
  if (!project || !regions) return;
  const left = regions[0]!;
  const track = trackOf(project, left.id);
  const next = track?.regions
    .filter((r) => r.id !== left.id && r.content.type === "audio" && r.start >= left.start && r.start <= left.start + left.length)
    .sort((a, b) => a.start - b.start)[0];
  if (!next) return notify("info", "The region needs a touching or overlapping audio region after it.");
  await useProject.getState().apply({ type: "createCrossfade", left: left.id, right: next.id, lengthMs });
}

async function transients(region: Region, sensitivity: number): Promise<number[] | null> {
  try {
    const ticks = await ipc.editing.transients(region.id, sensitivity);
    if (!ticks.length) notify("info", "No transients found; try a higher sensitivity.");
    return ticks.length ? ticks : null;
  } catch (e) {
    notifyError(toIpcError(e), "Transient detection failed");
    return null;
  }
}

export async function sliceAtTransients(sensitivity = 0.5) {
  const regions = audioSelection();
  if (!regions) return;
  const region = regions[0]!;
  const ticks = await transients(region, sensitivity);
  if (!ticks) return;
  const out = await useProject.getState().apply({ type: "sliceRegion", id: region.id, positions: ticks });
  if (out) notify("success", `Sliced into ${out.createdRegions.length + 1} regions (the file is unchanged).`);
}

export async function markersAtTransients(sensitivity = 0.5) {
  const regions = audioSelection();
  if (!regions) return;
  const ticks = await transients(regions[0]!, sensitivity);
  if (!ticks) return;
  const commands = ticks.slice(0, 256).map((tick, i) => ({ type: "addMarker" as const, tick, name: `Hit ${i + 1}` }));
  await useProject.getState().apply({ type: "batch", commands });
}

export async function bounceSelected(includeFader = false) {
  const project = currentProject();
  const regions = audioSelection();
  if (!project || !regions) return;
  const track = trackOf(project, regions[0]!.id);
  if (!track) return;
  const ids = regions.filter((r) => trackOf(project, r.id)?.id === track.id).map((r) => r.id);
  notify("info", "Bouncing…");
  try {
    const r = await enqueue(() => ipc.editing.bounce(track.id, ids, includeFader, true));
    useProject.getState().setView(r.view);
    useUi.getState().selectRegions(r.outcome.createdRegions, r.outcome.createdTracks);
    notify("success", "Bounced to a new track; the source regions are muted, not deleted.");
  } catch (e) {
    notifyError(toIpcError(e), "Bounce failed");
  }
}

/** Bounces a whole track (pre-fader) to a new track. */
export async function bounceTrack(trackId: string) {
  notify("info", "Bouncing…");
  try {
    const r = await enqueue(() => ipc.editing.bounce(trackId, null, false, true));
    useProject.getState().setView(r.view);
    useUi.getState().selectRegions(r.outcome.createdRegions, r.outcome.createdTracks);
    notify("success", "Bounced to a new track; the source regions are muted, not deleted.");
  } catch (e) {
    notifyError(toIpcError(e), "Bounce failed");
  }
}

export async function freezeTrack(trackId: string) {
  notify("info", "Freezing…");
  try {
    const r = await enqueue(() => ipc.editing.freeze(trackId));
    useProject.getState().setView(r.view);
    notify("success", "Frozen: the track plays its rendered file. Unfreeze to edit its regions or inserts.");
  } catch (e) {
    notifyError(toIpcError(e), "Freeze failed");
  }
}

export function unfreezeTrack(trackId: string) {
  return useProject.getState().apply({ type: "unfreezeTrack", id: trackId });
}
