import { describe, expect, it } from "vitest";
import golden from "../../crates/elyvon-core/tests/fixtures/grid_golden.json";
import type { GridSpec } from "../bindings/GridSpec";
import { gridPoints, snapTick, TempoMap } from "./tempoMap";

// The same vectors are asserted by `crates/elyvon-core/tests/grid_golden.rs`.
const map = new TempoMap(golden.tempo, golden.tempoChanges, golden.timeSignature, golden.meterChanges);

describe("TempoMap mirror matches the native core", () => {
  it("places bar lines across metre changes", () => {
    expect(golden.barStarts.map((_, b) => map.barStart(b))).toEqual(golden.barStarts);
  });

  it("formats positions and clock times", () => {
    golden.ticks.forEach((t, i) => {
      expect(map.formatPosition(t)).toBe(golden.positions[i]);
      expect(map.secondsAt(t)).toBeCloseTo(golden.seconds[i]!, 9);
      expect(map.tickAtSeconds(map.secondsAt(t))).toBeCloseTo(t, 6);
    });
  });

  it("lists beats and downbeats", () => {
    expect(map.beats(0, 12_000).map((b) => [b.tick, b.downbeat])).toEqual(golden.beats);
  });

  it("snaps with every division, triplets and swing", () => {
    for (const s of golden.snaps) {
      const spec = s.spec as GridSpec;
      expect(golden.ticks.map((t) => snapTick(map, t, spec)), JSON.stringify(spec)).toEqual(s.snapped);
      expect(gridPoints(map, 0, 12_000, spec), JSON.stringify(spec)).toEqual(s.points);
    }
  });
});
