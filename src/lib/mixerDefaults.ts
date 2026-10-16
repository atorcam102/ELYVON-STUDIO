// Neutral mixer values matching the native serde defaults (schema 4). Used
// by tests and by the UI when it needs a placeholder before the first view.

import type { Mixer } from "../bindings/Mixer";
import type { StripRouting } from "../bindings/StripRouting";

export function defaultStrip(): StripRouting {
  return { output: { type: "master" }, sends: [], inserts: [], soloSafe: false, group: null, format: "auto" };
}

export function emptyMixer(): Mixer {
  return { auxes: [], groups: [], folders: [], master: { volumeDb: 0, muted: false, inserts: [] }, panLaw: "constantPower" };
}
