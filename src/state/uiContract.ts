// Presentation only. Never part of .elyvon or an audio-thread message.
// Layout persistence is independently versioned by Rust Settings.schemaVersion.
import type { Layout } from "../bindings/Layout";
export type SettingsSection = "general" | "updates" | "audio" | "shortcuts";

export const UI_CONTRACT_VERSION = 1 as const;

export interface Selection {
  trackIds: string[];
  regionIds: string[];
  /** Anchor for shift-click range selection of tracks. */
  anchorTrackId: string | null;
}

export interface UiPresentationV1 {
  layout: Layout;
  pxPerQuarter: number;
  /** Edit cursor position in ticks (there is no playback yet). */
  cursorTicks: number;
  selection: Selection;
  settingsOpen: SettingsSection | null;
  aboutOpen: boolean;
  updateOpen: boolean;
  /** Simulated meter animation in the mixer (off by default). */
  demoMeters: boolean;
}
