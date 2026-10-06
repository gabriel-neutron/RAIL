// The one DPR rule for every canvas in the app.
//
// Rule: a canvas's backing store is its CSS size times an explicit
// `pixelRatio`, rounded to whole device pixels, and the 2d context is
// pre-transformed by that ratio so all drawing code works in CSS pixels.
//
// One documented exception: the two streaming canvases (waterfall row and
// spectrum curve) pass `pixelRatio` 1. `drawWaterfallRow` runs a per-pixel
// LUT loop plus a full-canvas blit on every frame, and docs/PERF.md §1 sets
// an explicit ~1 ms/frame NO-GO threshold measured without DPR; both the LUT
// loop and the ImageData row scale with backing-store width, so a 2x display
// would cost 2x there. The ratio is an argument rather than an assumption so
// that the exception is greppable at its two call sites.

/// Backing-store dimensions for a canvas of `cssWidth` x `cssHeight` at
/// `pixelRatio`. Whole pixels, never below 1 — fractional canvas dimensions
/// produce subpixel sampling that softens thin lines and text.
export const backingStoreSize = (
  cssWidth: number,
  cssHeight: number,
  pixelRatio: number,
): { width: number; height: number } => ({
  width: Math.max(1, Math.round(cssWidth * pixelRatio)),
  height: Math.max(1, Math.round(cssHeight * pixelRatio)),
});

export type PreparedCanvas = {
  ctx: CanvasRenderingContext2D;
  /// The width the caller must draw against — read once here so no caller
  /// re-reads `clientWidth` mid-draw and drifts from the sized buffer.
  cssWidthPx: number;
};

/// Size `canvas`'s backing store for `cssHeightPx` at `pixelRatio`, get its
/// 2d context and pre-transform it by the ratio.
///
/// `cssHeightPx` is caller-supplied rather than measured: the overlays draw
/// against their own height constants and every tick, label and bracket
/// offset is hardcoded against them.
///
/// `contextAttributes` is forwarded to `getContext` — the streaming canvases
/// need `{ alpha: false }` for their opaque blit path.
///
/// Returns `null` when the canvas has no layout width yet or no 2d context
/// is available.
export const prepareCanvas2d = (
  canvas: HTMLCanvasElement,
  cssHeightPx: number,
  pixelRatio: number,
  contextAttributes?: CanvasRenderingContext2DSettings,
): PreparedCanvas | null => {
  const cssWidthPx = canvas.clientWidth;
  if (cssWidthPx <= 0) return null;

  const { width, height } = backingStoreSize(cssWidthPx, cssHeightPx, pixelRatio);
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;

  const ctx = canvas.getContext("2d", contextAttributes);
  if (!ctx) return null;
  ctx.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
  return { ctx, cssWidthPx };
};
