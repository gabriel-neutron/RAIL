// The one place the spectrum's coordinate systems are defined.
//
// Three spaces meet on the spectrum stack: FFT bin index, real Hz, and
// canvas pixels. Every overlay used to rebuild the Hz<->pixel mapping
// privately inside its own draw effect; they now all ask this module.
//
// See: docs/DSP.md §9 for the transform itself, its bin/pixel cell
// semantics, and the known nominal-vs-true span discrepancy.

import { cellLeftX, xToCellIndex } from "./cellAxis";

/// The frequency span visible on screen, in Hz, at `zoom`.
///
/// This is the NOMINAL span the overlays label. The waterfall's true span
/// is slightly narrower at non-integer zoom because `cropCenter` keeps a
/// whole number of bins — see docs/DSP.md §9 "known discrepancy".
export const spanHz = (sampleRateHz: number, zoom: number): number =>
  sampleRateHz / zoom;

/// The Hz<->pixel mapping for one canvas width, at one tuning.
///
/// All widths and x coordinates are in the same space the caller draws in:
/// CSS pixels for the overlays (which run under `setTransform(dpr, …)`),
/// backing-store pixels for the streaming canvases, and
/// `getBoundingClientRect().width` for hit-testing.
export type SpectrumViewport = Readonly<{
  /// Visible span in Hz.
  spanHz: number;
  /// Frequency at canvas x = 0.
  minHz: number;
  /// Frequency at canvas x = width.
  maxHz: number;
  /// Absolute: real Hz -> canvas x.
  hzToX: (hz: number) => number;
  /// Absolute: canvas x -> real Hz. Exact inverse of `hzToX`.
  xToHz: (x: number) => number;
  /// Relative: a width in Hz -> a width in pixels. No centre term.
  hzWidthToPx: (widthHz: number) => number;
  /// Relative: a width in pixels -> a width in Hz. No centre term, which
  /// is what makes it safe inside a drag handler that retunes as it goes.
  pxWidthToHz: (widthPx: number) => number;
}>;

export type SpectrumViewportInput = {
  /// Tuned centre frequency in Hz — the frequency at canvas centre after
  /// the fs/4 shift (docs/DSP.md §1–3).
  centerHz: number;
  sampleRateHz: number;
  zoom: number;
  /// Canvas width in the caller's own pixel space.
  cssWidthPx: number;
};

/// Build the viewport for one draw, or `null` when the inputs cannot
/// describe a visible span (zero-width canvas, unset sample rate, bad zoom).
/// Callers bail on `null` instead of repeating the finite-span guard.
export const createSpectrumViewport = ({
  centerHz,
  sampleRateHz,
  zoom,
  cssWidthPx,
}: SpectrumViewportInput): SpectrumViewport | null => {
  if (!Number.isFinite(cssWidthPx) || cssWidthPx <= 0) return null;
  if (!Number.isFinite(centerHz)) return null;
  const span = spanHz(sampleRateHz, zoom);
  if (!Number.isFinite(span) || span <= 0) return null;

  const minHz = centerHz - span / 2;
  const maxHz = centerHz + span / 2;

  return Object.freeze({
    spanHz: span,
    minHz,
    maxHz,
    hzToX: (hz: number) => ((hz - minHz) / span) * cssWidthPx,
    xToHz: (x: number) => minHz + (x / cssWidthPx) * span,
    hzWidthToPx: (widthHz: number) => (widthHz / span) * cssWidthPx,
    pxWidthToHz: (widthPx: number) => (widthPx / cssWidthPx) * span,
  });
};

// The bin helpers below name the shared cell map (`./cellAxis`) in bin terms.
// The asymmetry between the two directions is the point, and is explained in
// docs/DSP.md §9.3.
//
// `binCount` is an input, never derived here: `cropCenter` in the Waterfall
// stays the sole owner of what is on screen, and the FFT size belongs to the
// Rust side.

/// Left edge, in pixels, of bin `binIndex` of `binCount` across `widthPx`.
/// The point map the spectrum polyline places its vertices with.
export const binLeftX = (
  binIndex: number,
  binCount: number,
  widthPx: number,
): number => cellLeftX(binIndex, binCount, widthPx);

/// The bin covering pixel column `x`. The cell map the waterfall row reads
/// each column through. Not clamped: callers iterate x over `[0, widthPx)`,
/// where the result is always in range.
export const xToBinIndex = (
  x: number,
  binCount: number,
  widthPx: number,
): number => xToCellIndex(x, binCount, widthPx);
