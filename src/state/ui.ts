// Presentation state: layout, zoom, cursor, selection, dialogs.
// None of this is part of the project model.

import { create } from "zustand";
import type { Selection, SettingsSection, UiPresentationV1 } from "./uiContract";
import type { GridSpec } from "../bindings/GridSpec";
import type { Layout } from "../bindings/Layout";
import type { Project } from "../bindings/Project";
import { clampZoom, DEFAULT_PX_PER_QUARTER } from "../lib/geometry";

export type BottomTab = "mixer" | "pianoRoll" | "audioEditor" | "score" | "smartTempo" | "smartControls";
export type BrowserTab = "files" | "loops" | "instruments" | "plugins";
export type { SettingsSection } from "./uiContract";

export type { Selection } from "./uiContract";

interface UiState extends UiPresentationV1 {
  /** User-chosen snap grid; `null` follows the zoom level. */
  snapSpec: GridSpec | null;
  setSnapSpec: (spec: GridSpec | null) => void;
  setLayout: (patch: Partial<Layout>) => void;
  replaceLayout: (layout: Layout) => void;
  setZoom: (pxPerQuarter: number) => void;
  setCursor: (ticks: number) => void;
  selectTracks: (ids: string[], anchor?: string | null) => void;
  selectRegions: (ids: string[], trackIds?: string[]) => void;
  clearSelection: () => void;
  /** Drops ids that no longer exist after the project changed. */
  pruneSelection: (project: Project) => void;
  openSettings: (section?: SettingsSection) => void;
  closeSettings: () => void;
  setAboutOpen: (open: boolean) => void;
  setUpdateOpen: (open: boolean) => void;
  setDemoMeters: (on: boolean) => void;
}

export const DEFAULT_LAYOUT: Layout = {
  inspectorVisible: true,
  browserVisible: true,
  bottomVisible: false,
  inspectorWidth: 296,
  browserWidth: 404,
  bottomHeight: 480,
  trackHeaderWidth: 408,
  bottomTab: "pianoRoll",
  browserTab: "instruments",
};

const EMPTY_SELECTION: Selection = { trackIds: [], regionIds: [], anchorTrackId: null };

export const useUi = create<UiState>((set) => ({
  layout: DEFAULT_LAYOUT,
  pxPerQuarter: DEFAULT_PX_PER_QUARTER,
  cursorTicks: 0,
  selection: EMPTY_SELECTION,
  settingsOpen: null,
  aboutOpen: false,
  updateOpen: false,
  demoMeters: false,
  snapSpec: null,
  setSnapSpec: (snapSpec) => set({ snapSpec }),

  setLayout: (patch) => set((s) => ({ layout: { ...s.layout, ...patch } })),
  replaceLayout: (layout) => set({ layout }),
  setZoom: (pxPerQuarter) => set({ pxPerQuarter: clampZoom(pxPerQuarter) }),
  setCursor: (ticks) => set({ cursorTicks: Math.max(0, Math.round(ticks)) }),
  selectTracks: (ids, anchor) =>
    set((s) => ({
      selection: {
        trackIds: ids,
        regionIds: [],
        anchorTrackId: anchor === undefined ? (ids[ids.length - 1] ?? s.selection.anchorTrackId) : anchor,
      },
    })),
  selectRegions: (ids, trackIds) =>
    set((s) => ({
      selection: {
        regionIds: ids,
        trackIds: trackIds ?? s.selection.trackIds,
        anchorTrackId: trackIds?.[trackIds.length - 1] ?? s.selection.anchorTrackId,
      },
    })),
  clearSelection: () => set({ selection: EMPTY_SELECTION }),
  pruneSelection: (project) =>
    set((s) => {
      const tracks = new Set(project.tracks.map((t) => t.id));
      const regions = new Set(project.tracks.flatMap((t) => t.regions.map((r) => r.id)));
      const trackIds = s.selection.trackIds.filter((id) => tracks.has(id));
      const regionIds = s.selection.regionIds.filter((id) => regions.has(id));
      if (trackIds.length === s.selection.trackIds.length && regionIds.length === s.selection.regionIds.length) {
        return s;
      }
      const anchor = s.selection.anchorTrackId;
      return {
        selection: { trackIds, regionIds, anchorTrackId: anchor && tracks.has(anchor) ? anchor : null },
      };
    }),
  openSettings: (section = "general") => set({ settingsOpen: section }),
  closeSettings: () => set({ settingsOpen: null }),
  setAboutOpen: (aboutOpen) => set({ aboutOpen }),
  setUpdateOpen: (updateOpen) => set({ updateOpen }),
  setDemoMeters: (demoMeters) => set({ demoMeters }),
}));

/**
 * Track ids selected by a click on `id`, following desktop conventions:
 * plain click selects one, Mod-click toggles, Shift-click selects a range.
 */
export function nextTrackSelection(
  order: string[],
  current: Selection,
  id: string,
  mode: "single" | "toggle" | "range",
): { ids: string[]; anchor: string } {
  if (mode === "toggle") {
    const has = current.trackIds.includes(id);
    return { ids: has ? current.trackIds.filter((t) => t !== id) : [...current.trackIds, id], anchor: id };
  }
  if (mode === "range" && current.anchorTrackId) {
    const a = order.indexOf(current.anchorTrackId);
    const b = order.indexOf(id);
    if (a >= 0 && b >= 0) {
      const [lo, hi] = a < b ? [a, b] : [b, a];
      return { ids: order.slice(lo, hi + 1), anchor: current.anchorTrackId };
    }
  }
  return { ids: [id], anchor: id };
}
