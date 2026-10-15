import type { TrackKind } from "../../bindings/TrackKind";
import type { IconName } from "../../components/Icon";

export const KIND_ICON: Record<TrackKind, IconName> = { audio: "audio", instrument: "instrument", midi: "midi" };
export const KIND_LABEL: Record<TrackKind, string> = { audio: "Audio", instrument: "Instrument", midi: "MIDI" };

// Mirrors `TRACK_PALETTE` in elyvon-project (new tracks get these in order).
export const TRACK_COLORS = [
  "#5B8DEF",
  "#3FB99C",
  "#E0A13B",
  "#D95D7A",
  "#9B7BE8",
  "#6CC25A",
  "#E5734A",
  "#4BB4D6",
  "#C78BD8",
  "#B8A76A",
] as const;
