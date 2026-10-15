import type { TrackKind } from "../../bindings/TrackKind";
import type { IconName } from "../../components/Icon";

export const KIND_ICON: Record<TrackKind, IconName> = { audio: "audio", instrument: "instrument", midi: "midi" };
export const KIND_LABEL: Record<TrackKind, string> = { audio: "Audio", instrument: "Instrument", midi: "MIDI" };

// Mirrors `TRACK_PALETTE` in elyvon-project (new tracks get these in order).
export const TRACK_COLORS = [
  "#365DCD",
  "#19A7C0",
  "#22BCBC",
  "#6445D5",
  "#2F6CB4",
  "#3FA75A",
  "#C99A22",
  "#D96F2B",
  "#C9455A",
  "#8E4FD1",
] as const;
