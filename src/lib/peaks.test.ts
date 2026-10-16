import { describe, expect, it } from "vitest";
import { decodePeaks } from "./ipc";

describe("waveform wire format", () => {
  it("decodes u32 column count and i16 min/max pairs", () => {
    const buf = new ArrayBuffer(4 + 2 * 4 * 2);
    const v = new DataView(buf);
    v.setUint32(0, 2, true);
    [-32767, 32767, -16384, 16384, 0, 0, -1, 1].forEach((x, i) => v.setInt16(4 + i * 2, x, true));
    const p = decodePeaks(buf);
    expect(p.columns).toBe(2);
    [-1, 1, -16384 / 32767, 16384 / 32767].forEach((x, i) => expect(p.data[i]).toBeCloseTo(x, 6));
  });

  it("tolerates truncated buffers", () => {
    const buf = new ArrayBuffer(6);
    new DataView(buf).setUint32(0, 100, true);
    expect(decodePeaks(buf).data.length).toBe(1);
  });
});
