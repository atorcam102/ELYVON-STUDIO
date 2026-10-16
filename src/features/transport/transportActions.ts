// Transport actions. Each one sends a command to the native engine; the
// playhead the UI draws comes back from engine events (`state/transport`).

import { useMidi } from "../../state/midi";
import { useCapture } from "../../state/capture";
import { ipc } from "../../lib/ipc";
import { engineAvailable, useApp } from "../../state/app";
import { notify, notifyError } from "../../state/notifications";
import { currentProject, useProject } from "../../state/project";
import { isRolling, useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";

function ready(): boolean {
  if (engineAvailable(useApp.getState().engine)) return true;
  const engine = useApp.getState().engine;
  notify("warning", engine?.state === "unavailable" ? engine.reason : "The audio engine is not running.");
  return false;
}

/** Moves the edit cursor and the engine playhead. */
export function locate(tick: number) {
  const t = Math.max(0, Math.round(tick));
  useUi.getState().setCursor(t);
  if (engineAvailable(useApp.getState().engine)) void useTransport.getState().send({ type: "locate", tick: t });
}

export function play() {
  if (!ready()) return;
  const project = currentProject();
  const t = project?.transport;
  void useTransport.getState().send({
    type: "play",
    fromTick: useUi.getState().cursorTicks,
    countIn: (t?.countInBars ?? 0) > 0,
    preRoll: (t?.preRollBars ?? 0) > 0,
  });
}

/** Stop; a second stop returns to where playback started (engine rule). */
export function stop() {
  if (useMidi.getState().status?.recording) { void useMidi.getState().stop(); return; }
  if (useCapture.getState().status?.active) { void useCapture.getState().stop(); return; }
  if (!ready()) return;
  void useTransport.getState().send({ type: "stop" });
}

export function pause() {
  if (!ready()) return;
  void useTransport.getState().send({ type: "pause" });
}

/** Space bar: play when stopped, stop when rolling. */
export function togglePlay() {
  if (useMidi.getState().status?.recording) { void useMidi.getState().stop(); return; }
  if (useCapture.getState().status?.active) { void useCapture.getState().stop(); return; }
  if (isRolling(useTransport.getState().snapshot)) stop();
  else play();
}

export function toggleMetronome() {
  const p = currentProject();
  if (!p) return;
  const m = p.transport.metronome;
  void useProject.getState().apply({ type: "setMetronome", settings: { ...m, enabled: !m.enabled } });
}

export function toggleCountIn() {
  const p = currentProject();
  if (!p) return;
  void useProject.getState().apply({ type: "setCountIn", bars: p.transport.countInBars > 0 ? 0 : 1 });
}

/** Tap tempo, timed by the native monotonic clock; applies after 3 taps. */
export async function tapTempo() {
  try {
    const bpm = await ipc.transport.tap();
    if (bpm !== null && currentProject()?.transport.tempo !== bpm) {
      await useProject.getState().apply({ type: "setTempo", bpm });
    }
  } catch (e) {
    notifyError(e, "Tap tempo failed");
  }
}

// When playback stops, the edit cursor takes the engine's final position.
useTransport.subscribe((s, prev) => {
  if (isRolling(prev.snapshot) && s.snapshot && !isRolling(s.snapshot)) {
    useUi.getState().setCursor(Math.round(s.snapshot.ticks));
  }
});
