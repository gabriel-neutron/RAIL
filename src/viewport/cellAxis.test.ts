import { describe, expect, it } from "vitest";

import {
  cellCenterX,
  cellLeftX,
  xToCellIndex,
  xToCellIndexClamped,
} from "./cellAxis";

describe("cellAxis", () => {
  // The defect this module exists to close: the strip drew cells with i/n but
  // hit-tested with round(ratio * (n - 1)), a point map, so a click at 70 %
  // across ten steps selected step 6 while the pixel under the cursor was 7.
  it("selects the cell the cursor is actually over", () => {
    const n = 10;
    const w = 512;
    expect(xToCellIndexClamped(0.7 * w, n, w)).toBe(7);
    expect(Math.round(0.7 * (n - 1))).toBe(6);
  });

  it("round-trips every cell centre back to its own index", () => {
    const w = 512;
    for (let n = 1; n <= 64; n += 1) {
      for (let i = 0; i < n; i += 1) {
        expect(xToCellIndex(cellCenterX(i, n, w), n, w)).toBe(i);
      }
    }
  });

  it("tiles the width with no gap or overlap", () => {
    const n = 17;
    const w = 512;
    expect(cellLeftX(0, n, w)).toBe(0);
    expect(cellLeftX(n, n, w)).toBe(w);
    for (let i = 0; i < n; i += 1) {
      expect(cellLeftX(i + 1, n, w)).toBeGreaterThan(cellLeftX(i, n, w));
    }
  });

  it("gives the edge cells the same click width as the interior", () => {
    const n = 8;
    const w = 512;
    const cellW = w / n;
    // Just inside each end of the first and last cell.
    expect(xToCellIndex(0, n, w)).toBe(0);
    expect(xToCellIndex(cellW - 1, n, w)).toBe(0);
    expect(xToCellIndex(w - cellW, n, w)).toBe(n - 1);
    expect(xToCellIndex(w - 1, n, w)).toBe(n - 1);
  });

  it("clamps clicks outside the strip", () => {
    expect(xToCellIndexClamped(-40, 10, 512)).toBe(0);
    expect(xToCellIndexClamped(9999, 10, 512)).toBe(9);
  });

  it("returns 0 rather than NaN for a degenerate strip", () => {
    expect(xToCellIndexClamped(10, 0, 512)).toBe(0);
    expect(xToCellIndexClamped(10, 10, 0)).toBe(0);
  });

  it("agrees with the unclamped form inside the strip", () => {
    for (const x of [0, 1, 63, 255, 511]) {
      expect(xToCellIndexClamped(x, 10, 512)).toBe(xToCellIndex(x, 10, 512));
    }
  });
});
