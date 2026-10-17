import { describe, expect, it } from "vitest";
import { panelResize } from "./panelResize";

describe("panel resize stop", () => {
  it("holds at the minimum until the pointer crosses the extra travel", () => {
    expect(panelResize(180, 180, 550, 48)).toEqual({ size: 180, collapsed: false });
    expect(panelResize(133, 180, 550, 48)).toEqual({ size: 180, collapsed: false });
    expect(panelResize(132, 180, 550, 48)).toEqual({ size: 180, collapsed: true });
  });
  it("allows reversing at the stop and caps expansion", () => {
    expect(panelResize(220, 180, 550, 48)).toEqual({ size: 220, collapsed: false });
    expect(panelResize(900, 180, 550, 48)).toEqual({ size: 550, collapsed: false });
  });
  it("never collapses panels without a collapse action", () => {
    expect(panelResize(-100, 180, 550)).toEqual({ size: 180, collapsed: false });
  });
});
