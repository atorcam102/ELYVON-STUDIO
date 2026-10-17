import { describe, expect, it } from "vitest";
import golden from "../../crates/elyvon-project/tests/fixtures/automation_golden.json";
import type { AutomationLane } from "../bindings/AutomationLane";
import { fromNorm, lanePieces, laneValueAt, laneValueBefore, overlay, paramSpec, piecesValue, toNorm } from "./automation";

// The same vectors are asserted by `crates/elyvon-project/tests/automation_golden.rs`.
const lanes = golden.lanes as AutomationLane[];

describe("automation mirror", () => {
  it("evaluates lanes exactly like the native core", () => {
    lanes.forEach((lane, li) => {
      golden.ticks.forEach((t, i) => {
        expect(laneValueAt(lane, t), `lane ${li} at ${t}`).toBeCloseTo(golden.values[li]![i]!, 9);
        expect(laneValueBefore(lane, t), `lane ${li} before ${t}`).toBeCloseTo(golden.before[li]![i]!, 9);
      });
    });
  });

  it("overlays region lanes inside their region", () => {
    const base = lanePieces(lanes[0]!)!;
    const over = lanePieces(golden.region as AutomationLane, golden.regionStart)!;
    const merged = overlay(base, over, golden.regionStart, golden.regionEnd);
    golden.ticks.forEach((t, i) => expect(piecesValue(merged, t)).toBeCloseTo(golden.overlaidNorm[i]!, 9));
  });

  it("round-trips values through the normalised scale", () => {
    const cutoff = paramSpec({ type: "insert", id: "x", param: "filterCutoff" });
    expect(fromNorm(cutoff, 0.5)).toBeCloseTo(Math.sqrt(20 * 20000), 6);
    for (const v of [20, 440, 20000]) expect(fromNorm(cutoff, toNorm(cutoff, v))).toBeCloseTo(v, 6);
    expect(fromNorm(paramSpec({ type: "mute" }), 0.6)).toBe(1);
  });
});
