import { describe, expect, it } from "vitest";
import type { EffectInfo } from "../bindings/EffectInfo";
import type { EffectParamInfo } from "../bindings/EffectParamInfo";
import { bandParamVisible, clampParam, eqBands, formatParam, fromPosition, newSlot, paramValue, plainParams, setCatalogForTests, effectInfo, toPosition } from "./effects";

const param = (p: Partial<EffectParamInfo>): EffectParamInfo => ({
  index: 0,
  key: "x",
  label: "X",
  unit: "none",
  min: 0,
  max: 1,
  default: 0,
  scale: "linear",
  choices: [],
  automatable: true,
  ...p,
});

describe("effect parameter mapping", () => {
  it("maps logarithmic parameters on a log axis and back", () => {
    const f = param({ unit: "hz", min: 20, max: 20000, default: 1000, scale: "log" });
    expect(toPosition(f, 20)).toBe(0);
    expect(toPosition(f, 20000)).toBe(1);
    expect(toPosition(f, Math.sqrt(20 * 20000))).toBeCloseTo(0.5, 6);
    expect(fromPosition(f, 0.5)).toBe(632);
    expect(fromPosition(f, 2)).toBe(20000);
  });

  it("rounds choices and clamps out-of-range values", () => {
    const c = param({ scale: "choice", min: 0, max: 3, choices: ["a", "b", "c", "d"] });
    expect(clampParam(c, 2.6)).toBe(3);
    expect(clampParam(c, -4)).toBe(0);
    expect(clampParam(c, Number.NaN)).toBe(0);
    expect(formatParam(c, 1)).toBe("b");
  });

  it("formats values with Spanish decimals and units", () => {
    expect(formatParam(param({ unit: "hz", min: 20, max: 20000 }), 2500)).toBe("2,50 kHz");
    expect(formatParam(param({ unit: "db", min: -24, max: 24 }), -3.25)).toBe("-3,3 dB");
    expect(formatParam(param({ unit: "ratio", min: 1, max: 20 }), 4)).toBe("4,00:1");
    expect(formatParam(param({ unit: "percent", min: 0, max: 100 }), 35)).toBe("35 %");
    expect(formatParam(param({ scale: "toggle" }), 1)).toBe("Sí");
  });
});

describe("equaliser helpers", () => {
  const params: EffectParamInfo[] = [];
  ["On", "Type", "Freq", "Gain", "Q", "Slope"].forEach((n, i) => params.push(param({ index: i, key: `b1${n}`, label: `Banda 1 ${n}` })));
  params.push(param({ index: 6, key: "outputDb", label: "Salida", unit: "db", min: -24, max: 24 }));
  const info: EffectInfo = { kind: "parametricEq", label: "EQ", short: "EQ", category: "eq", hasSidechain: false, params };

  it("groups band parameters and keeps the rest apart", () => {
    expect(eqBands(info)).toHaveLength(1);
    expect(eqBands(info)[0]!.map((p) => p.key)).toEqual(["b1On", "b1Type", "b1Freq", "b1Gain", "b1Q", "b1Slope"]);
    expect(plainParams(info).map((p) => p.key)).toEqual(["outputDb"]);
  });

  it("shows gain on bells and shelves, slope on cuts", () => {
    expect(bandParamVisible(2, "b3Gain")).toBe(true);
    expect(bandParamVisible(0, "b1Gain")).toBe(false);
    expect(bandParamVisible(0, "b1Slope")).toBe(true);
    expect(bandParamVisible(2, "b3Slope")).toBe(false);
    expect(bandParamVisible(5, "b3Q")).toBe(true);
  });

  it("builds default slots from the catalogue", () => {
    setCatalogForTests([info]);
    expect(effectInfo("parametricEq")).toBe(info);
    const slot = newSlot(info);
    expect(slot.kind).toBe("parametricEq");
    expect(paramValue(slot, params[6]!)).toBe(0);
    expect(paramValue({ ...slot, params: {} }, params[6]!)).toBe(0);
  });
});
