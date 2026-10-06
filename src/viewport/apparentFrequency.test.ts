import { describe, expect, it } from "vitest";

import { ZOOM_MAX } from "../store/radio";
import { createSpectrumViewport, cropWindow, retuneShiftPx } from "./spectrumViewport";

// SYNTHETIC: a carrier at a known absolute frequency, not captured data.
// Rust half of the same sweep: src-tauri/src/dsp/ghost_sweep.rs (issue #24).
const SYNTHETIC_CARRIER_HZ = 100_300_000;
const SAMPLE_RATE_HZ = 2_048_000;
const FFT_SIZE = 8192;
const BIN_HZ = SAMPLE_RATE_HZ / FFT_SIZE;
const CANVAS_WIDTH_PX = 1000;
const CENTER_OFFSETS_HZ = [-90_000, -40_000, -7_000, -300, 0, 300, 7_000, 40_000, 90_000];
const ZOOMS = [1, 2, 3, 6, 7.5, 16, 50, ZOOM_MAX];

/// Where the viewport says the carrier is, after the DSP puts it in bin
/// `N/2 + (carrier - centre) / binHz`, the waterfall crops the frame and draws
/// the peak's bin at its pixel cell. `null` when the crop cuts the carrier off.
const apparentHzAfterCrop = (centerHz: number, zoom: number): number | null => {
  const peakBin = FFT_SIZE / 2 + Math.round((SYNTHETIC_CARRIER_HZ - centerHz) / BIN_HZ);
  const { start, kept } = cropWindow(FFT_SIZE, zoom);
  if (peakBin < start || peakBin >= start + kept) return null;
  const view = createSpectrumViewport({
    centerHz,
    sampleRateHz: SAMPLE_RATE_HZ,
    zoom,
    fftSize: FFT_SIZE,
    cssWidthPx: CANVAS_WIDTH_PX,
  });
  if (view === null) throw new Error("expected a viewport");
  const peakCellCentreX = ((peakBin - start + 0.5) * CANVAS_WIDTH_PX) / kept;
  return view.xToHz(peakCellCentreX);
};

describe("apparent frequency after the frontend crop", () => {
  it("equals the true frequency within one bin at every centre and zoom", () => {
    let checked = 0;
    for (const zoom of ZOOMS) {
      for (const offset of CENTER_OFFSETS_HZ) {
        const centerHz = SYNTHETIC_CARRIER_HZ + offset;
        const apparent = apparentHzAfterCrop(centerHz, zoom);
        if (apparent === null) continue;
        checked += 1;
        expect(
          Math.abs(apparent - SYNTHETIC_CARRIER_HZ),
          `zoom ${zoom}, centre ${offset >= 0 ? "+" : ""}${offset} Hz from the carrier`,
        ).toBeLessThanOrEqual(BIN_HZ);
      }
    }
    expect(checked).toBeGreaterThan(40);
  });
});

// A waterfall row keeps the pixel column it was painted at. A retune that does
// not move the history leaves the old streak labelled with the NEW centre: the
// ghost of issue #24, displaced by (new centre - old centre).
describe("history rows after a retune", () => {
  const viewAt = (centerHz: number, zoom: number) => {
    const view = createSpectrumViewport({
      centerHz,
      sampleRateHz: SAMPLE_RATE_HZ,
      zoom,
      fftSize: FFT_SIZE,
      cssWidthPx: CANVAS_WIDTH_PX,
    });
    if (view === null) throw new Error("expected a viewport");
    return view;
  };

  it("keeps an old row on the carrier's true frequency once shifted", () => {
    for (const zoom of [1, 4, ZOOM_MAX]) {
      for (const [oldOffset, newOffset] of [[-60_000, -30_000], [-30_000, 0], [20_000, -5_000]]) {
        const oldCenterHz = SYNTHETIC_CARRIER_HZ + oldOffset;
        const newCenterHz = SYNTHETIC_CARRIER_HZ + newOffset;
        const oldView = viewAt(oldCenterHz, zoom);
        const newView = viewAt(newCenterHz, zoom);
        const paintedX = oldView.hzToX(SYNTHETIC_CARRIER_HZ);
        const shiftedX = paintedX + retuneShiftPx(newView, oldCenterHz, newCenterHz);
        const apparentHz = newView.xToHz(shiftedX);
        const oneColumnHz = newView.pxWidthToHz(1);
        expect(Math.abs(apparentHz - SYNTHETIC_CARRIER_HZ)).toBeLessThanOrEqual(oneColumnHz);
        // Unshifted, the same row is mislabelled by the full centre change.
        const unshiftedHz = newView.xToHz(paintedX);
        expect(unshiftedHz - SYNTHETIC_CARRIER_HZ).toBeCloseTo(newCenterHz - oldCenterHz, 3);
      }
    }
  });
});
