import { create } from "zustand";

export type SidePanel = "events" | "markers" | "tempo" | "signature" | "files" | "notes" | "loops";
export const useChrome = create<{
  sidePanel: SidePanel | null;
  controlsVisible: boolean;
  dialog: "recording" | "midi" | "transport" | null;
  setDialog: (dialog: "recording" | "midi" | "transport" | null) => void;
  setSidePanel: (panel: SidePanel | null) => void;
  toggleSidePanel: (panel: SidePanel) => void;
  toggleControls: () => void;
}>((set) => ({
  sidePanel: null,
  controlsVisible: true,
  dialog: null,
  setDialog: (dialog) => set({ dialog }),
  setSidePanel: (sidePanel) => set({ sidePanel }),
  toggleSidePanel: (panel) => set(s => ({ sidePanel: s.sidePanel === panel ? null : panel })),
  toggleControls: () => set(s => ({ controlsVisible: !s.controlsVisible })),
}));
