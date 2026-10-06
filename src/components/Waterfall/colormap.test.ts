import { describe, expect, it } from "vitest";

import { buildColormapLut } from "./colormap";

const luma = (lut: Uint8ClampedArray, i: number): number =>
  0.2126 * lut[i * 3] + 0.7152 * lut[i * 3 + 1] + 0.0722 * lut[i * 3 + 2];

describe("buildColormapLut", () => {
  it("packs one RGB triplet per entry", () => {
    expect(buildColormapLut(256)).toHaveLength(256 * 3);
  });

  it("anchors the first entry on the cold tube and the last on bloom", () => {
    const lut = buildColormapLut(256);
    expect([lut[0], lut[1], lut[2]]).toEqual([4, 3, 1]);
    expect([lut[765], lut[766], lut[767]]).toEqual([255, 246, 226]);
  });

  it("does not divide by zero for a single-entry LUT", () => {
    const lut = buildColormapLut(1);
    expect(lut).toHaveLength(3);
    expect([lut[0], lut[1], lut[2]]).toEqual([4, 3, 1]);
  });

  it("stays within the 0-255 byte range across the ramp", () => {
    for (const v of buildColormapLut(512)) {
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThanOrEqual(255);
    }
  });

  // The waterfall encodes one ordered quantity, so the ramp must be ordered
  // too: any dip in luminance would read as less signal where there is more.
  it("increases in luminance at every step", () => {
    const lut = buildColormapLut(256);
    for (let i = 1; i < 256; i += 1) {
      expect(luma(lut, i)).toBeGreaterThan(luma(lut, i - 1));
    }
  });

  it("interpolates continuously, with no banding between stops", () => {
    const lut = buildColormapLut(256);
    for (let i = 1; i < 256; i += 1) {
      expect(luma(lut, i) - luma(lut, i - 1)).toBeLessThan(4);
    }
  });

  it("spans nearly the full luminance range", () => {
    const lut = buildColormapLut(256);
    expect(luma(lut, 0)).toBeLessThan(10);
    expect(luma(lut, 255)).toBeGreaterThan(230);
  });
});
