import { useEffect, useMemo, useRef, useState } from "react";

import { useWaterfall } from "../../hooks/useWaterfall";
import { useCaptureStore } from "../../store/capture";
import { useRadioStore } from "../../store/radio";
import { useReplayStore } from "../../store/replay";
import FilterBandMarker from "../FilterBandMarker";
import FrequencyAxis from "../FrequencyAxis";
import Spectrum from "../Spectrum";
import { buildColormapLut } from "./colormap";

const WATERFALL_HEIGHT = 360;
const SPECTRUM_HEIGHT = 90;
const DB_FLOOR_DEFAULT = -100;
const DB_CEIL_DEFAULT = -20;
/// Cumulative pointer-motion threshold (px) that distinguishes a
/// click-to-tune from a pan-to-retune gesture.
const DRAG_THRESHOLD_PX = 4;
/// Background fill used when the drag shift exposes a blank strip
/// on the waterfall canvas. Matches `.waterfall-canvas`'s CSS bg.
const WATERFALL_BG = "#07090c";

/// Dev-only: `localStorage.setItem("rail_profile_waterfall", "1")` then reload.
/// Logs rolling averages for LUT vs canvas blit (see docs/PERF.md).
const waterfallProfileEnabled = (): boolean =>
  import.meta.env.DEV &&
  typeof localStorage !== "undefined" &&
  localStorage.getItem("rail_profile_waterfall") === "1";

type WaterfallProps = {
  enabled?: boolean;
  onAudio?: (frame: Float32Array) => void;
};

/// Crop the center `len/zoom` bins of a shifted FFT frame. After the
/// `fs/4` digital mixer + FFT shift, the user's target sits at bin
/// `N/2`, so a symmetric slice around the middle keeps the tuned
/// signal centered at any zoom level (docs/DSP.md §1–3).
const cropCenter = (frame: Float32Array, zoom: number): Float32Array => {
  if (zoom <= 1) return frame;
  const kept = Math.max(16, Math.floor(frame.length / zoom));
  const start = Math.floor((frame.length - kept) / 2);
  return frame.subarray(start, start + kept);
};

/// Apply a per-bin exponential moving average in the dB domain.
/// `alpha` = 1.0 passes the frame through unchanged (no averaging).
/// `alpha` < 1.0 blends the new frame into the running average:
///   avg[i] = alpha × frame[i] + (1 − alpha) × avg[i]
/// Lower alpha = more smoothing = slower response to rapid changes.
function applyEma(
  rawFrame: Float32Array,
  avgRef: React.MutableRefObject<Float32Array | null>,
  alpha: number,
): Float32Array {
  if (alpha >= 1.0) return rawFrame;
  const prev = avgRef.current;
  if (!prev || prev.length !== rawFrame.length) {
    const init = new Float32Array(rawFrame);
    avgRef.current = init;
    return init;
  }
  const oneMinusAlpha = 1.0 - alpha;
  for (let i = 0; i < prev.length; i += 1) {
    prev[i] = alpha * rawFrame[i] + oneMinusAlpha * prev[i];
  }
  return prev;
}

export const Waterfall = ({ enabled = true, onAudio }: WaterfallProps) => {
  const waterfallCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const spectrumCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const rowImageRef = useRef<ImageData | null>(null);
  const lut = useMemo(() => buildColormapLut(256), []);

  const [dbFloor, setDbFloor] = useState(DB_FLOOR_DEFAULT);
  const [dbCeil, setDbCeil] = useState(DB_CEIL_DEFAULT);
  const dbFloorRef = useRef(dbFloor);
  const dbCeilRef = useRef(dbCeil);
  useEffect(() => { dbFloorRef.current = dbFloor; }, [dbFloor]);
  useEffect(() => { dbCeilRef.current = dbCeil; }, [dbCeil]);

  const handleFloorChange = (v: number) => setDbFloor(Math.min(v, dbCeil - 10));
  const handleCeilChange = (v: number) => setDbCeil(Math.max(v, dbFloor + 10));

  // Smooth = 0 → alpha 1.0 (raw, no averaging).
  // Smooth = 65 → alpha 0.35 (balanced default).
  // Smooth = 95 → alpha 0.05 (very smooth, slow to respond).
  const [smooth, setSmooth] = useState(65);
  const smoothRef = useRef(1.0 - smooth / 100);
  useEffect(() => { smoothRef.current = 1.0 - smooth / 100; }, [smooth]);
  const avgFrameRef = useRef<Float32Array | null>(null);

  const zoom = useRadioStore((s) => s.zoom);
  const sampleRateHz = useRadioStore((s) => s.sampleRateHz);
  /// Bumped by the replay store on open / seek / loop. While replaying
  /// an IQ file we want the waterfall's Y-axis to track file time, not
  /// wall-clock emit order — clearing the canvas on each discontinuity
  /// makes the painted region grow from the seek point forward.
  const waterfallEpoch = useReplayStore((s) => s.waterfallEpoch);

  const zoomRef = useRef(zoom);
  useEffect(() => {
    zoomRef.current = zoom;
  }, [zoom]);

  // Scroll wheel zoom. React's synthetic onWheel is passive by
  // default, so `preventDefault()` inside a synthetic handler is a
  // no-op — attach a native listener with `passive: false` instead.
  useEffect(() => {
    const canvas = waterfallCanvasRef.current;
    if (!canvas) return;
    const handler = (e: WheelEvent) => {
      e.preventDefault();
      if (e.deltaY === 0) return;
      const factor = e.deltaY < 0 ? 1.25 : 1 / 1.25;
      const store = useRadioStore.getState();
      store.setZoom(store.zoom * factor);
    };
    canvas.addEventListener("wheel", handler, { passive: false });
    return () => canvas.removeEventListener("wheel", handler);
  }, []);

  // Flag consumed by the `onFrame` callback below. While the user is
  // drag-panning the waterfall we skip row draws so the CSS-shifted
  // old content stays coherent (new rows would paint at the wrong
  // canvas-X relative to the transform and tear visibly). Spectrum
  // keeps updating so the user still sees live magnitude.
  const isDraggingRef = useRef(false);

  const { session, error } = useWaterfall({
    enabled,
    onAudio,
    onFrame: (rawFrame) => {
      const smoothed = applyEma(rawFrame, avgFrameRef, smoothRef.current);
      const frame = cropCenter(smoothed, zoomRef.current);
      if (!isDraggingRef.current) {
        drawWaterfallRow(
          waterfallCanvasRef.current,
          frame,
          rowImageRef,
          lut,
          dbFloorRef.current,
          dbCeilRef.current,
        );
      }
      drawSpectrum(spectrumCanvasRef.current, frame, dbFloorRef.current, dbCeilRef.current);
    },
  });

  useEffect(() => {
    const canvas = waterfallCanvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) return;
    ctx.fillStyle = "#07090c";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    // Reset per-bin averaging and row buffer so the new frame length
    // is used on the next draw (session change, zoom change, or seek).
    rowImageRef.current = null;
    avgFrameRef.current = null;
  }, [session?.fftSize, zoom, waterfallEpoch]);

  // Register a PNG screenshot source with the capture store so the
  // "save screenshot" menu entry can grab the waterfall without
  // reaching into component refs.
  useEffect(() => {
    const provider = () =>
      new Promise<Blob | null>((resolve) => {
        const canvas = waterfallCanvasRef.current;
        if (!canvas) {
          resolve(null);
          return;
        }
        canvas.toBlob((blob) => resolve(blob), "image/png");
      });
    useCaptureStore.getState().setScreenshotProvider(provider);
    return () => {
      useCaptureStore.getState().setScreenshotProvider(null);
    };
  }, []);

  // Pointer lifecycle for pan-to-retune with click-to-tune fallback.
  // Under 4 px of cumulative movement the gesture is treated as a
  // click; above that the drag has already retuned via repeated
  // setFrequency calls (debounced in the store).
  const dragStateRef = useRef<{
    startX: number;
    startHz: number;
    moved: number;
  } | null>(null);
  const [isDragging, setIsDragging] = useState(false);

  const pxToOffsetHz = (px: number, rectWidth: number): number => {
    const store = useRadioStore.getState();
    const displayedSpan = store.sampleRateHz / store.zoom;
    return (px / rectWidth) * displayedSpan;
  };

  const handlePointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0) return;
    const canvas = waterfallCanvasRef.current;
    if (!canvas) return;
    const store = useRadioStore.getState();
    if (!store.streaming) return;
    canvas.setPointerCapture(e.pointerId);
    dragStateRef.current = {
      startX: e.clientX,
      startHz: store.frequencyHz,
      moved: 0,
    };
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const drag = dragStateRef.current;
    if (drag === null) return;
    const canvas = waterfallCanvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0) return;
    const deltaPx = e.clientX - drag.startX;
    drag.moved = Math.max(drag.moved, Math.abs(deltaPx));
    if (drag.moved < DRAG_THRESHOLD_PX) return;
    if (!isDragging) {
      setIsDragging(true);
      isDraggingRef.current = true;
    }
    // Drag right = content right = tuned center lower, so negate the
    // px→Hz mapping to feel like panning a map. Live retune updates
    // the axis + spectrum + marker; CSS transform visually shifts
    // the cached waterfall rows to follow the cursor.
    const deltaHz = -pxToOffsetHz(deltaPx, rect.width);
    useRadioStore.getState().setFrequency(drag.startHz + deltaHz);
    canvas.style.transform = `translateX(${deltaPx}px)`;
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const drag = dragStateRef.current;
    if (drag === null) return;
    dragStateRef.current = null;
    const wasDragging = isDraggingRef.current;
    isDraggingRef.current = false;
    setIsDragging(false);
    const canvas = waterfallCanvasRef.current;
    if (canvas && canvas.hasPointerCapture(e.pointerId)) {
      canvas.releasePointerCapture(e.pointerId);
    }

    if (wasDragging && canvas) {
      // Bake the CSS translation into the pixel buffer so the cached
      // history stays aligned with the new tuned center once the
      // transform is removed. Without this, resetting transform to 0
      // would snap the old rows back to their original canvas X,
      // breaking continuity with rows that arrive post-release.
      const deltaPxScreen = e.clientX - drag.startX;
      const rect = canvas.getBoundingClientRect();
      const scale = rect.width > 0 ? canvas.width / rect.width : 1;
      const deltaPxCanvas = Math.round(deltaPxScreen * scale);
      if (deltaPxCanvas !== 0) {
        shiftCanvasContent(canvas, deltaPxCanvas);
      }
      canvas.style.transform = "";
      return;
    }

    // Click-to-tune fallback: tap without crossing the drag
    // threshold. After `fs/4` shift the signal sits at canvas
    // center, so pixel X maps linearly across the span.
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0) return;
    const store = useRadioStore.getState();
    if (!store.streaming) return;
    const offsetPx = e.clientX - rect.left - rect.width / 2;
    store.setFrequency(store.frequencyHz + pxToOffsetHz(offsetPx, rect.width));
  };

  const displayedSpanHz = sampleRateHz / zoom;

  return (
    <section className="waterfall">
      <div className="waterfall-header">
        <div className="waterfall-status">
          {error && (
            <span className="waterfall-error">stream error: {error}</span>
          )}
          {!error && session === null && (
            <span className="waterfall-pending">opening stream…</span>
          )}
          {!error && session && (
            <span className="waterfall-ok">
              fs={(session.sampleRateHz / 1e6).toFixed(3)} MHz · N=
              {session.fftSize} · span=
              {(displayedSpanHz / 1e6).toFixed(3)} MHz · zoom=
              {zoom.toFixed(1)}x
            </span>
          )}
        </div>
        <div className="wf-range-controls">
          <span className="wf-range-label">Floor</span>
          <input
            type="range"
            className="wf-range-slider"
            min={-130}
            max={-30}
            step={5}
            value={dbFloor}
            onChange={(e) => handleFloorChange(Number(e.target.value))}
            aria-label="Waterfall floor dBFS"
          />
          <span className="wf-range-value">{dbFloor}</span>
          <span className="wf-range-label">Ceil</span>
          <input
            type="range"
            className="wf-range-slider"
            min={-80}
            max={0}
            step={5}
            value={dbCeil}
            onChange={(e) => handleCeilChange(Number(e.target.value))}
            aria-label="Waterfall ceiling dBFS"
          />
          <span className="wf-range-value">{dbCeil}</span>
          <span className="wf-range-label wf-range-sep">|</span>
          <span className="wf-range-label">Smooth</span>
          <input
            type="range"
            className="wf-range-slider"
            min={0}
            max={95}
            step={5}
            value={smooth}
            onChange={(e) => setSmooth(Number(e.target.value))}
            aria-label="Waterfall temporal smoothing"
          />
          <span className="wf-range-value">{smooth}%</span>
        </div>
      </div>
      <div className="spectrum-wrap">
        <Spectrum ref={spectrumCanvasRef} />
        <FrequencyAxis />
        <FilterBandMarker />
      </div>
      <canvas
        ref={waterfallCanvasRef}
        className={
          isDragging ? "waterfall-canvas is-dragging" : "waterfall-canvas"
        }
        height={WATERFALL_HEIGHT}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
      />
    </section>
  );
};

/// Shift the entire canvas content horizontally by `deltaPxCanvas`
/// (positive = right, negative = left) using an offscreen snapshot
/// so self-overlap is well-defined. Exposed strip is filled with
/// the waterfall background color.
function shiftCanvasContent(
  canvas: HTMLCanvasElement,
  deltaPxCanvas: number,
): void {
  const ctx = canvas.getContext("2d", { alpha: false });
  if (!ctx) return;
  const snapshot = document.createElement("canvas");
  snapshot.width = canvas.width;
  snapshot.height = canvas.height;
  const sctx = snapshot.getContext("2d");
  if (!sctx) return;
  sctx.drawImage(canvas, 0, 0);
  ctx.fillStyle = WATERFALL_BG;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(snapshot, deltaPxCanvas, 0);
}

/// Scroll the waterfall down by one row and paint the new top row.
function drawWaterfallRow(
  canvas: HTMLCanvasElement | null,
  frame: Float32Array,
  rowImageRef: React.MutableRefObject<ImageData | null>,
  lut: Uint8ClampedArray,
  dbFloor: number,
  dbCeil: number,
): void {
  if (!canvas) return;
  const ctx = canvas.getContext("2d", { alpha: false });
  if (!ctx) return;

  // Fill the full CSS-rendered width so zoom never produces a tiny
  // buffer that the browser has to upscale. clientWidth is 0 before
  // first layout; fall back to the current attribute width in that case.
  const targetW = canvas.clientWidth > 0 ? canvas.clientWidth : canvas.width;
  if (canvas.width !== targetW) {
    canvas.width = targetW;
  }
  if (canvas.height !== WATERFALL_HEIGHT) {
    canvas.height = WATERFALL_HEIGHT;
  }

  if (
    rowImageRef.current === null ||
    rowImageRef.current.width !== canvas.width
  ) {
    rowImageRef.current = ctx.createImageData(canvas.width, 1);
  }

  const row = rowImageRef.current;
  const pixels = row.data;
  const span = dbCeil - dbFloor;
  const lutEntries = lut.length / 3;
  const profile = waterfallProfileEnabled();
  const t0 = profile ? performance.now() : 0;
  const binCount = frame.length;
  const canvasW = canvas.width;
  for (let x = 0; x < canvasW; x += 1) {
    const binIdx = Math.floor((x * binCount) / canvasW);
    const normalized = Math.max(
      0,
      Math.min(1, (frame[binIdx] - dbFloor) / span),
    );
    const lutIdx = (normalized * (lutEntries - 1)) | 0;
    const offset = lutIdx * 3;
    const out = x * 4;
    pixels[out] = lut[offset];
    pixels[out + 1] = lut[offset + 1];
    pixels[out + 2] = lut[offset + 2];
    pixels[out + 3] = 255;
  }
  const t1 = profile ? performance.now() : 0;

  ctx.drawImage(
    canvas,
    0,
    0,
    canvas.width,
    canvas.height - 1,
    0,
    1,
    canvas.width,
    canvas.height - 1,
  );
  ctx.putImageData(row, 0, 0);
  if (profile) {
    const t2 = performance.now();
    const acc = drawWaterfallRowProfAccum;
    acc.lut += t1 - t0;
    acc.blit += t2 - t1;
    acc.n += 1;
    if (acc.n >= 60) {
      console.info(
        "[rail waterfall profile] avg ms / frame — lut:",
        (acc.lut / acc.n).toFixed(3),
        "blit:",
        (acc.blit / acc.n).toFixed(3),
        `(n=${acc.n}, bins=${frame.length})`,
      );
      acc.lut = 0;
      acc.blit = 0;
      acc.n = 0;
    }
  }
}

const drawWaterfallRowProfAccum = { lut: 0, blit: 0, n: 0 };

/// Draw a dB-scaled magnitude curve (filled under the line) on the
/// spectrum canvas. Uses the same `[dbFloor, dbCeil]` range as the
/// waterfall colormap so the two views read consistently.
function drawSpectrum(
  canvas: HTMLCanvasElement | null,
  frame: Float32Array,
  dbFloor: number,
  dbCeil: number,
): void {
  if (!canvas) return;
  const ctx = canvas.getContext("2d", { alpha: true });
  if (!ctx) return;

  const targetW = canvas.clientWidth > 0 ? canvas.clientWidth : canvas.width;
  if (canvas.width !== targetW) {
    canvas.width = targetW;
  }
  if (canvas.height !== SPECTRUM_HEIGHT) {
    canvas.height = SPECTRUM_HEIGHT;
  }

  const w = canvas.width;
  const h = canvas.height;
  ctx.clearRect(0, 0, w, h);

  const span = dbCeil - dbFloor;
  const toY = (db: number): number => {
    const n = Math.max(0, Math.min(1, (db - dbFloor) / span));
    return h - n * h;
  };
  const binToX = (i: number): number => (i / frame.length) * w;

  // Filled area under the curve.
  const gradient = ctx.createLinearGradient(0, 0, 0, h);
  gradient.addColorStop(0, "rgba(58, 160, 255, 0.55)");
  gradient.addColorStop(1, "rgba(58, 160, 255, 0.04)");
  ctx.fillStyle = gradient;
  ctx.beginPath();
  ctx.moveTo(0, h);
  for (let i = 0; i < frame.length; i += 1) {
    ctx.lineTo(binToX(i), toY(frame[i]));
  }
  ctx.lineTo(w, h);
  ctx.closePath();
  ctx.fill();

  // Curve on top.
  ctx.strokeStyle = "rgba(156, 205, 255, 0.9)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let i = 0; i < frame.length; i += 1) {
    const y = toY(frame[i]);
    if (i === 0) ctx.moveTo(binToX(i), y);
    else ctx.lineTo(binToX(i), y);
  }
  ctx.stroke();
}

export default Waterfall;
