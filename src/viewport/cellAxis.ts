// Equal-width cell geometry: the one cell map the whole app draws against.
//
// Two consumers share it. The waterfall lays FFT bins across a canvas width;
// the scanner's band-activity strip lays scan steps across its own. Neither
// axis is in Hz — an index is divided by a count and scaled to a width — so
// this module takes no centre, sample rate or zoom, and the spectrum
// viewport's bin helpers are named wrappers over it.
//
// See: docs/DSP.md §9.3 for the point-map / cell-map distinction and why the
// two directions are asymmetric.

/// Left edge, in pixels, of cell `index` of `cellCount` across `widthPx`.
/// `cellLeftX(cellCount)` is the right edge of the last cell, so consecutive
/// calls tile the width with no gap or overlap.
export const cellLeftX = (
  index: number,
  cellCount: number,
  widthPx: number,
): number => (index / cellCount) * widthPx;

/// Horizontal centre, in pixels, of cell `index`.
export const cellCenterX = (
  index: number,
  cellCount: number,
  widthPx: number,
): number => ((index + 0.5) / cellCount) * widthPx;

/// The cell covering pixel column `x`. Not clamped: callers that iterate `x`
/// over `[0, widthPx)` are always in range, and pay nothing for a guard.
export const xToCellIndex = (
  x: number,
  cellCount: number,
  widthPx: number,
): number => Math.floor((x * cellCount) / widthPx);

/// The cell covering pixel column `x`, clamped to `[0, cellCount - 1]`.
/// For hit-testing, where `x` comes from a pointer and can sit outside the
/// element. Returns 0 for a degenerate strip (no cells, or no width).
export const xToCellIndexClamped = (
  x: number,
  cellCount: number,
  widthPx: number,
): number => {
  if (cellCount <= 0 || widthPx <= 0) return 0;
  return Math.max(0, Math.min(cellCount - 1, xToCellIndex(x, cellCount, widthPx)));
};
