import { describe, expect, it } from "vitest";
import { TempoMap } from "../../lib/tempoMap";
import { scoreLayout, staffStep } from "./scoreModel";

describe("score model", () => {
  it("spells pitches on diatonic staff steps", () => {
    expect(staffStep(60)).toEqual({ step: 35, sharp: false }); // C4
    expect(staffStep(61)).toEqual({ step: 35, sharp: true }); // C#4
    expect(staffStep(64).step).toBe(37); // E4
    expect(staffStep(43).step).toBe(25); // G2
    expect(staffStep(71).step - staffStep(60).step).toBe(6); // B4 above C4
  });

  it("lays out the bars a region spans, following metre changes", () => {
    const map = new TempoMap(120, [], { numerator: 4, denominator: 4 }, [{ bar: 2, timeSignature: { numerator: 3, denominator: 4 } }]);
    const region = { start: 3840, length: 3840 + 2880 * 2 } as never;
    const layout = scoreLayout(map, region, 153 + 4 * 210, 50);
    expect(layout.firstBar).toBe(1);
    expect(layout.totalBars).toBe(3);
    expect(layout.visible).toBe(4);
    expect(layout.barWidth).toBeCloseTo(210);
  });
});
