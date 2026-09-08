import { describe, expect, it } from "vitest";

import { ZOOM_MAX, ZOOM_MIN } from "../store/radio";
import {
  binLeftX,
  createSpectrumViewport,
  spanHz,
  xToBinIndex,
  type SpectrumViewport,
} from "./spectrumViewport";

const CENTER_HZ = 101_100_000;
const SAMPLE_RATE_HZ = 2_048_000;

const build = (zoom: number, cssWidthPx: number): SpectrumViewport => {
  const view = createSpectrumViewport({
    centerHz: CENTER_HZ,
    sampleRateHz: SAMPLE_RATE_HZ,
    zoom,
    cssWidthPx,
  });
  if (view === null) throw new Error("expected a viewport");
  return view;
};

const ZOOMS = [1, 2, 7.5, 64];
// 577 is prime, so a width of 577 catches any assumption that the canvas
// width divides evenly into bins or ticks.
const WIDTHS = [100, 577, 1920];

describe("spanHz", () => {
  // Characterisation: the overlays label the NOMINAL span. The waterfall's
  // true span is fs * kept / N (docs/DSP.md §9). Changing this is a product
  // decision, so it has a test to change.
  it("is the nominal sample rate over zoom", () => {
    expect(spanHz(2_048_000, 1)).toBe(2_048_000);
    expect(spanHz(2_048_000, 50)).toBe(2_048_000 / 50);
  });
});

describe("createSpectrumViewport", () => {
  it("round-trips x -> Hz -> x across the visible span", () => {
    for (const zoom of ZOOMS) {
      for (const cssWidthPx of WIDTHS) {
        const view = build(zoom, cssWidthPx);
        for (let k = 0; k <= 20; k += 1) {
          const hz = view.minHz + (view.spanHz * k) / 20;
          const back = view.xToHz(view.hzToX(hz));
          expect(Math.abs(back - hz) / hz).toBeLessThan(1e-9);
        }
      }
    }
  });

  it("anchors the edges and the centre", () => {
    for (const zoom of ZOOMS) {
      for (const cssWidthPx of WIDTHS) {
        const view = build(zoom, cssWidthPx);
        expect(view.hzToX(view.minHz)).toBeCloseTo(0, 9);
        expect(view.hzToX(view.maxHz)).toBeCloseTo(cssWidthPx, 9);
        expect(view.hzToX(CENTER_HZ)).toBeCloseTo(cssWidthPx / 2, 9);
      }
    }
  });

  it("spans symmetrically about the tuned centre", () => {
    const view = build(4, 800);
    expect(view.spanHz).toBe(SAMPLE_RATE_HZ / 4);
    expect(view.maxHz - view.minHz).toBeCloseTo(view.spanHz, 6);
    expect((view.minHz + view.maxHz) / 2).toBeCloseTo(CENTER_HZ, 6);
  });

  // Pan safety. The drag handler retunes as it moves, so it must use a
  // RELATIVE mapping: an absolute xToHz would make each move change minHz,
  // which changes the next move's mapping — a runaway pan.
  it("maps pixel widths to Hz widths independently of the centre", () => {
    const a = build(8, 640);
    const b = createSpectrumViewport({
      centerHz: CENTER_HZ + 5_000_000,
      sampleRateHz: SAMPLE_RATE_HZ,
      zoom: 8,
      cssWidthPx: 640,
    });
    if (b === null) throw new Error("expected a viewport");
    for (const px of [-320, -1, 0, 17, 640]) {
      expect(b.pxWidthToHz(px)).toBe(a.pxWidthToHz(px));
    }
  });

  it("round-trips a width through Hz and back to pixels", () => {
    const view = build(7.5, 577);
    for (const px of [1, 42, 288.5, 577]) {
      expect(view.hzWidthToPx(view.pxWidthToHz(px))).toBeCloseTo(px, 9);
    }
  });

  it("returns null for inputs that cannot describe a visible span", () => {
    const base = { centerHz: CENTER_HZ, sampleRateHz: SAMPLE_RATE_HZ, zoom: 1, cssWidthPx: 800 };
    for (const cssWidthPx of [0, -1, Number.NaN]) {
      expect(createSpectrumViewport({ ...base, cssWidthPx })).toBeNull();
    }
    for (const sampleRateHz of [0, Number.NaN]) {
      expect(createSpectrumViewport({ ...base, sampleRateHz })).toBeNull();
    }
    for (const zoom of [0, Number.POSITIVE_INFINITY]) {
      expect(createSpectrumViewport({ ...base, zoom })).toBeNull();
    }
    expect(createSpectrumViewport({ ...base, centerHz: Number.NaN })).toBeNull();
  });

  it("builds across the store's whole zoom range", () => {
    for (const zoom of [ZOOM_MIN, 3, 17.5, ZOOM_MAX]) {
      const view = build(zoom, 800);
      expect(view.spanHz).toBeGreaterThan(0);
    }
  });
});

describe("bin helpers", () => {
  it("maps a bin's left edge back to that bin exactly", () => {
    for (const binCount of [16, 1024, 8192]) {
      for (const widthPx of [577, 1001, 1920]) {
        for (let i = 0; i < binCount; i += 1) {
          expect(xToBinIndex(binLeftX(i, binCount, widthPx), binCount, widthPx)).toBe(i);
        }
      }
    }
  });

  // The other direction is a cell map, not a point map: a pixel column maps
  // to the bin covering it, whose left edge is at or before that column.
  // Documented asymmetry, not a rounding bug — see docs/DSP.md §9.
  it("lands within one bin width in the reverse direction", () => {
    const binCount = 1024;
    const widthPx = 577;
    const binWidthPx = widthPx / binCount;
    for (let x = 0; x < widthPx; x += 1) {
      const left = binLeftX(xToBinIndex(x, binCount, widthPx), binCount, widthPx);
      expect(left).toBeLessThanOrEqual(x);
      expect(x - left).toBeLessThan(binWidthPx);
    }
  });

  it("tiles the width from 0 to the full width", () => {
    expect(binLeftX(0, 512, 800)).toBe(0);
    expect(binLeftX(512, 512, 800)).toBe(800);
  });
});
