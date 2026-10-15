import { describe, expect, it } from "vitest";
import { dbToFaderPos, faderPosToDb, formatDb, formatPan, MAX_DB, MIN_DB, stripProjectExtension } from "./format";
import { clampZoom, gridFor, pxToTicks, snap, ticksToPx, zoomAround, MAX_PX_PER_QUARTER } from "./geometry";
import { formatClock, formatKey, formatLength, formatPosition, noteName, positionFromTicks, PPQ, ticksPerBar } from "./music";
import { gridBackground } from "./gridBackground";

const FOUR_FOUR = { numerator: 4, denominator: 4 };
const SIX_EIGHT = { numerator: 6, denominator: 8 };

describe("music", () => {
  it("formats bar.beat.division.tick positions", () => {
    expect(formatPosition(0, FOUR_FOUR)).toBe("1.1.1.000");
    expect(formatPosition(PPQ, FOUR_FOUR)).toBe("1.2.1.000");
    expect(formatPosition(4 * PPQ + PPQ / 4 + 7, FOUR_FOUR)).toBe("2.1.2.007");
    expect(positionFromTicks(ticksPerBar(SIX_EIGHT), SIX_EIGHT)).toMatchObject({ bar: 2, beat: 1 });
    expect(positionFromTicks(-5, FOUR_FOUR).bar).toBe(1);
  });

  it("formats clock time from tempo", () => {
    expect(formatClock(0, 120)).toBe("00:00.000");
    expect(formatClock(PPQ * 2, 120)).toBe("00:01.000");
    expect(formatClock(PPQ * 150, 120)).toBe("01:15.000");
  });

  it("formats lengths and keys", () => {
    expect(formatLength(ticksPerBar(FOUR_FOUR) * 2 + PPQ, FOUR_FOUR)).toBe("2 bars 1 beat");
    expect(formatLength(10, FOUR_FOUR)).toBe("10 ticks");
    expect(formatKey({ tonic: "F#", mode: "minor" })).toBe("F# min");
    expect(noteName(60)).toBe("C4");
    expect(noteName(69)).toBe("A4");
  });
});

describe("geometry", () => {
  it("converts ticks and pixels symmetrically", () => {
    for (const ticks of [0, 1, 960, 123_456]) {
      expect(pxToTicks(ticksToPx(ticks, 28), 28)).toBeCloseTo(ticks, 9);
    }
  });

  it("chooses a finer snap grid when zoomed in", () => {
    const far = gridFor(FOUR_FOUR, 3);
    const near = gridFor(FOUR_FOUR, 200);
    expect(far.snapTicks).toBe(ticksPerBar(FOUR_FOUR));
    expect(far.beatPx).toBe(0);
    expect(near.snapTicks).toBe(PPQ / 4);
    expect(far.labelEvery).toBeGreaterThan(1);
    expect(near.labelEvery).toBe(1);
  });

  it("snaps to the nearest grid line", () => {
    expect(snap(470, 240)).toBe(480);
    expect(snap(100, 240)).toBe(0);
    expect(snap(17.4, 0)).toBe(17);
  });

  it("keeps the tick under the pointer fixed while zooming", () => {
    const before = { pxPerQuarter: 28, scrollLeft: 500 };
    const anchor = 300;
    const tickUnderPointer = pxToTicks(before.scrollLeft + anchor, before.pxPerQuarter);
    const after = zoomAround(before.pxPerQuarter, 1.5, before.scrollLeft, anchor);
    expect(pxToTicks(after.scrollLeft + anchor, after.pxPerQuarter)).toBeCloseTo(tickUnderPointer, 6);
    expect(clampZoom(10_000)).toBe(MAX_PX_PER_QUARTER);
  });

  it("builds tiled grid backgrounds and skips dense layers", () => {
    const bg = gridBackground([
      { axis: "x", every: 112, color: "red" },
      { axis: "x", every: 1, color: "blue" },
      { axis: "y", every: 56, color: "green" },
    ]);
    expect(bg.backgroundImage.split("linear-gradient").length - 1).toBe(2);
    expect(bg.backgroundSize).toBe("112px 100%, 100% 56px");
    expect(bg.backgroundRepeat).toBe("repeat-x, repeat-y");
  });
});

describe("format", () => {
  it("fader taper is monotonic and invertible", () => {
    let prev = -Infinity;
    for (let i = 0; i <= 100; i++) {
      const db = faderPosToDb(i / 100);
      expect(db).toBeGreaterThanOrEqual(prev);
      prev = db;
      if (db > MIN_DB) expect(dbToFaderPos(db)).toBeCloseTo(i / 100, 6);
    }
    expect(faderPosToDb(0)).toBe(MIN_DB);
    expect(faderPosToDb(1)).toBe(MAX_DB);
    expect(dbToFaderPos(0)).toBeGreaterThan(0.7);
  });

  it("formats dB and pan for display", () => {
    expect(formatDb(MIN_DB)).toBe("−∞");
    expect(formatDb(0)).toBe("0.0");
    expect(formatDb(-3.04)).toBe("−3.0");
    expect(formatDb(2.5)).toBe("+2.5");
    expect(formatPan(0)).toBe("C");
    expect(formatPan(-1)).toBe("L64");
    expect(formatPan(0.25)).toBe("R16");
  });

  it("strips the project extension", () => {
    expect(stripProjectExtension("Song.ELYVON")).toBe("Song");
    expect(stripProjectExtension("Song.mid")).toBe("Song.mid");
  });
});
