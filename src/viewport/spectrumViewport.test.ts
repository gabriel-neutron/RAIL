import { describe, expect, it } from "vitest";

import { ZOOM_MAX, ZOOM_MIN } from "../store/radio";
import {
  binLeftX,
  createSpectrumViewport,
  cropWindow,
  spanHz,
  xToBinIndex,
  type SpectrumViewport,
} from "./spectrumViewport";

const CENTER_HZ = 101_100_000;
const SAMPLE_RATE_HZ = 2_048_000;
const FFT_SIZE = 8192;
const BIN_HZ = SAMPLE_RATE_HZ / FFT_SIZE;

const build = (zoom: number, cssWidthPx: number): SpectrumViewport => {
  const view = createSpectrumViewport({
    centerHz: CENTER_HZ,
    sampleRateHz: SAMPLE_RATE_HZ,
    zoom,
    fftSize: FFT_SIZE,
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
  it("is the span the waterfall really shows, fs * kept / N", () => {
    expect(spanHz(2_048_000, 1, FFT_SIZE)).toBe(2_048_000);
    // floor(8192 / 50) = 163 bins, not the nominal 163.84.
    expect(spanHz(2_048_000, 50, FFT_SIZE)).toBe(163 * BIN_HZ);
  });
});

describe("cropWindow", () => {
  it("keeps the whole frame at zoom 1", () => {
    expect(cropWindow(FFT_SIZE, 1)).toEqual({ start: 0, kept: FFT_SIZE });
  });

  it("centres on the middle of bin N/2 whenever N - kept is odd", () => {
    // kept = 8192 / 64 = 128 is even, so N - kept is even: half-bin residual.
    // kept = floor(8192 / 7.5) = 1092 is even too; 8192 / 6 -> 1365 is odd.
    const { start, kept } = cropWindow(FFT_SIZE, 6);
    expect(kept).toBe(1365);
    expect(start + kept / 2).toBe(FFT_SIZE / 2 + 0.5);
  });

  it("never keeps fewer than 16 bins or more than the frame", () => {
    expect(cropWindow(FFT_SIZE, 1e9).kept).toBe(16);
    expect(cropWindow(8, 2).kept).toBeLessThanOrEqual(8);
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
        const halfBinPx = view.hzWidthToPx(BIN_HZ / 2);
        expect(Math.abs(view.hzToX(CENTER_HZ) - cssWidthPx / 2)).toBeLessThanOrEqual(
          halfBinPx + 1e-9,
        );
      }
    }
  });

  it("spans the true width, centred on the tuned bin to within half a bin", () => {
    const view = build(4, 800);
    expect(view.spanHz).toBe(SAMPLE_RATE_HZ / 4);
    expect(view.maxHz - view.minHz).toBeCloseTo(view.spanHz, 6);
    // Bin N/2 is centred on CENTER_HZ, so its edges sit half a bin either side.
    expect(view.hzToX(CENTER_HZ - BIN_HZ / 2)).toBeLessThan(view.hzToX(CENTER_HZ));
  });

  it("puts the centre of bin N/2 exactly on CENTER_HZ at every zoom", () => {
    // The bin that holds the tuned frequency, as an edge index and in pixels.
    for (const zoom of [1, 2, 3, 6, 7.5, 50, 64]) {
      const { start, kept } = cropWindow(FFT_SIZE, zoom);
      const view = build(zoom, kept * 4);
      const dcCentreX = (FFT_SIZE / 2 + 0.5 - start) * 4;
      expect(view.hzToX(CENTER_HZ)).toBeCloseTo(dcCentreX, 6);
    }
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
      fftSize: FFT_SIZE,
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
    const base = {
      centerHz: CENTER_HZ,
      sampleRateHz: SAMPLE_RATE_HZ,
      zoom: 1,
      fftSize: FFT_SIZE,
      cssWidthPx: 800,
    };
    for (const cssWidthPx of [0, -1, Number.NaN]) {
      expect(createSpectrumViewport({ ...base, cssWidthPx })).toBeNull();
    }
    for (const sampleRateHz of [0, Number.NaN]) {
      expect(createSpectrumViewport({ ...base, sampleRateHz })).toBeNull();
    }
    for (const zoom of [0, Number.POSITIVE_INFINITY]) {
      expect(createSpectrumViewport({ ...base, zoom })).toBeNull();
    }
    for (const fftSize of [0, Number.NaN]) {
      expect(createSpectrumViewport({ ...base, fftSize })).toBeNull();
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
