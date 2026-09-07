import { describe, expect, it } from "vitest";

import { buildColormapLut } from "./colormap";

describe("buildColormapLut", () => {
  it("packs one RGB triplet per entry", () => {
    expect(buildColormapLut(256)).toHaveLength(256 * 3);
  });

  it("anchors the first entry on the dark-blue stop and the last on red", () => {
    const lut = buildColormapLut(256);
    expect([lut[0], lut[1], lut[2]]).toEqual([8, 10, 40]);
    expect([lut[765], lut[766], lut[767]]).toEqual([230, 40, 30]);
  });

  it("does not divide by zero for a single-entry LUT", () => {
    const lut = buildColormapLut(1);
    expect(lut).toHaveLength(3);
    expect([lut[0], lut[1], lut[2]]).toEqual([8, 10, 40]);
  });

  it("stays within the 0-255 byte range across the ramp", () => {
    for (const v of buildColormapLut(512)) {
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThanOrEqual(255);
    }
  });

  it("interpolates continuously, with no banding between stops", () => {
    const lut = buildColormapLut(256);
    const luma = (i: number) =>
      0.2126 * lut[i * 3] + 0.7152 * lut[i * 3 + 1] + 0.0722 * lut[i * 3 + 2];
    // The ramp is not monotonic — cyan is brighter than green — but a lerped
    // LUT must never jump: a large step would show up as a visible band.
    for (let i = 1; i < 256; i += 1) {
      expect(Math.abs(luma(i) - luma(i - 1))).toBeLessThan(6);
    }
  });

  it("brightens from the noise floor to the mid-scale stops", () => {
    const lut = buildColormapLut(256);
    const luma = (i: number) =>
      0.2126 * lut[i * 3] + 0.7152 * lut[i * 3 + 1] + 0.0722 * lut[i * 3 + 2];
    expect(luma(128)).toBeGreaterThan(luma(0));
    expect(luma(255)).toBeGreaterThan(luma(0));
  });
});
