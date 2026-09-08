import { describe, expect, it } from "vitest";

import { backingStoreSize } from "./canvasSizing";

// `prepareCanvas2d` is deliberately untested here: it is the thin DOM shell
// around `backingStoreSize`, and jsdom returns null from getContext("2d")
// with no canvas package installed. Before adding a component-render test for
// any overlay, src/test/setup.ts needs a canvas 2d stub and a ResizeObserver
// polyfill — it currently provides neither.

describe("backingStoreSize", () => {
  it("is the identity at pixelRatio 1", () => {
    expect(backingStoreSize(800, 24, 1)).toEqual({ width: 800, height: 24 });
    expect(backingStoreSize(577, 360, 1)).toEqual({ width: 577, height: 360 });
  });

  it("scales by the ratio and rounds to whole device pixels", () => {
    expect(backingStoreSize(800, 24, 2)).toEqual({ width: 1600, height: 48 });
    expect(backingStoreSize(800, 26, 3)).toEqual({ width: 2400, height: 78 });
    expect(backingStoreSize(801, 25, 1.25)).toEqual({ width: 1001, height: 31 });
  });

  it("returns integers for every ratio", () => {
    for (const ratio of [1, 1.25, 2, 3]) {
      for (const css of [1, 7, 577.4, 1920]) {
        const { width, height } = backingStoreSize(css, css, ratio);
        expect(Number.isInteger(width)).toBe(true);
        expect(Number.isInteger(height)).toBe(true);
      }
    }
  });

  it("never sizes a buffer below one pixel", () => {
    expect(backingStoreSize(0, 0, 2)).toEqual({ width: 1, height: 1 });
    expect(backingStoreSize(0.2, 0.2, 1)).toEqual({ width: 1, height: 1 });
  });
});
