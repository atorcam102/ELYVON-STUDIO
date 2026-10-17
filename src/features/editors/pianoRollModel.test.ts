import { describe, expect, it } from "vitest";
import type { Project } from "../../bindings/Project";
import { defaultAutomation, defaultStrip, emptyMixer } from "../../lib/mixerDefaults";
import {
  collectRoll,
  createdKeys,
  hitTest,
  inScale,
  laneValue,
  moveCommand,
  notesInRect,
  visibleRange,
  type View,
} from "./pianoRollModel";

function project(noteCount = 3): Project {
  const notes = Array.from({ length: noteCount }, (_, i) => ({
    id: i + 1,
    pitch: 60 + (i % 12),
    velocity: 100,
    channel: 0,
    muted: false,
    start: i * 480,
    length: 400,
    quantizeOffset: 0,
  }));
  const region = (id: string, start: number) => ({
    capture: null,
    midiEvents: [{ tick: 0, bytes: [0xb0, 64, 127] }, { tick: 10, bytes: [0xe1, 0, 64] }],
    id,
    name: id,
    start,
    length: 3840 * 4,
    muted: false,
    content: { type: "midi" as const, notes },
    automation: [],
  });
  return {
    id: "p",
    name: "P",
    sampleRate: 48000,
    lengthBars: 64,
    transport: {
      tempo: 120,
      timeSignature: { numerator: 4, denominator: 4 },
      key: { tonic: "D", mode: "minor" },
      loopRange: { enabled: false, start: 0, end: 15360 },
      tempoChanges: [],
      meterChanges: [],
      metronome: { enabled: false, volumeDb: -6, accent: true },
      countInBars: 0,
      preRollBars: 0,
      punch: { enabled: false, start: 0, end: 15360 },
    },
    media: [],
    markers: [],
    mixer: emptyMixer(),
    tracks: [
      {
        io: { armed: false, inputChannel: 0, inputChannels: 1, monitor: false, midiInput: null, midiOutput: null, midiChannel: null, outputChannel: null, midiMonitor: true },
        strip: defaultStrip(),
        folder: null,
        id: "t",
        name: "Keys",
        kind: "instrument",
        color: "#365DCD",
        muted: false,
        soloed: false,
        volumeDb: 0,
        pan: 0,
        regions: [region("a", 0), region("b", 3840 * 8)],
        automation: defaultAutomation(),
        freeze: null,
        comps: [],
      },
    ],
  };
}

const view: View = { pxPerTick: 0.1, keyHeight: 10, scrollX: 0, scrollY: 0, width: 800, height: 600 };

describe("piano roll model", () => {
  it("collects notes of several regions in absolute time", () => {
    const roll = collectRoll(project(), ["b", "a"]);
    expect(roll.regions.map((r) => r.id)).toEqual(["a", "b"]);
    expect(roll.notes.length).toBe(6);
    expect(roll.notes[3]!.abs).toBe(3840 * 8);
    expect(roll.notes.every((n, i, all) => i === 0 || all[i - 1]!.abs <= n.abs)).toBe(true);
  });

  it("hit tests bodies and resize edges", () => {
    const roll = collectRoll(project(), ["a"]);
    // Note 1: pitch 60, ticks 0–400 → x 0–40, y (127-60)*10 = 670.
    const y = (127 - 60) * 10 + 5;
    expect(hitTest(roll, view, 20, y)?.zone).toBe("body");
    expect(hitTest(roll, view, 38, y)?.zone).toBe("end");
    expect(hitTest(roll, view, 1, y)?.zone).toBe("start");
    expect(hitTest(roll, view, 45, y)).toBeNull();
    expect(hitTest(roll, view, 20, y + 10)).toBeNull();
  });

  it("rubber-band selection and culling scale to 10 000 notes", () => {
    const roll = collectRoll(project(10_000), ["a"]);
    const t0 = performance.now();
    const [lo, hi] = visibleRange(roll, 480 * 5000, 480 * 5010);
    const hits = notesInRect(roll, 480 * 5000, 480 * 5010, 0, 127);
    const took = performance.now() - t0;
    expect(hi - lo).toBeLessThan(20);
    expect(hits.length).toBe(10);
    expect(took).toBeLessThan(5);
  });

  it("builds one undo step for multi-region edits and maps created ids", () => {
    const roll = collectRoll(project(), ["a", "b"]);
    const sel = [roll.notes[0]!, roll.notes[1]!, roll.notes[4]!];
    const cmd = moveCommand(sel, 240, 1, true);
    expect(cmd?.type).toBe("batch");
    expect(createdKeys(sel, [4, 5, 4])).toEqual(["a:4", "a:5", "b:4"]);
  });

  it("highlights the key's scale and reads controller lanes", () => {
    const key = { tonic: "D" as const, mode: "minor" as const };
    expect([62, 64, 65, 67, 69, 70, 72].every((p) => inScale(p, key))).toBe(true);
    expect(inScale(63, key)).toBe(false);
    expect(laneValue("cc64", 0, [0xb0, 64, 127])).toBe(127);
    expect(laneValue("cc64", 1, [0xb0, 64, 127])).toBeNull();
    expect(laneValue("pitchBend", 1, [0xe1, 0, 64])).toBe(8192);
  });
});
