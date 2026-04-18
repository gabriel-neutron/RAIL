// Scientific-instrument filter-passband indicator. Replaces the
// trapezoid skirt with a horizontal bracket (bar + end caps),
// a soft phosphor-cyan glow column across the passband, and a thin
// center-pointer with a diamond cap marking the tuned bin. The
// optional bandwidth label prints above the bracket when there's
// enough horizontal room.
//
// Read-only view of `bandwidthHz`, `sampleRateHz`, `zoom` — redraws
// only when one of those changes, not per waterfall frame.

import { useEffect, useRef, useState } from "react";

import { useRadioStore } from "../../store/radio";

const HEIGHT_PX = 26;
const ACCENT = "#7ee7ff";
const BAR_COLOR = "rgba(126, 231, 255, 0.55)";
const GLOW_TOP = "rgba(126, 231, 255, 0.10)";
const GLOW_BOTTOM = "rgba(126, 231, 255, 0.00)";
const CENTER_LINE_COLOR = "rgba(255, 255, 255, 0.8)";
const LABEL_COLOR = "rgba(126, 231, 255, 0.85)";

// Layout (top to bottom):
//   0..9    label band (rendered only when halfBwPx is wide enough)
//   14      bracket bar centerline
//   10..18  end-cap span (CAP_HEIGHT around BAR_Y)
const BAR_Y = 14;
const BAR_THICKNESS = 2;
const CAP_HEIGHT = 8;
const LABEL_MIN_HALF_PX = 24;
const LABEL_BASELINE = 9;
const DIAMOND_HALF = 2;

/// Format `bandwidthHz` into a short label. Mirrors FrequencyAxis's
/// unit choice so the two components read in the same language.
const formatBandwidth = (hz: number): string => {
  if (hz >= 1_000_000) {
    const digits = hz >= 10_000_000 ? 1 : 2;
    return `${(hz / 1_000_000).toFixed(digits)} MHz`;
  }
  if (hz >= 1_000) {
    const digits = hz >= 100_000 ? 0 : hz >= 10_000 ? 1 : 2;
    return `${(hz / 1_000).toFixed(digits)} kHz`;
  }
  return `${Math.round(hz)} Hz`;
};

export const FilterBandMarker = () => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const bandwidthHz = useRadioStore((s) => s.bandwidthHz);
  const sampleRateHz = useRadioStore((s) => s.sampleRateHz);
  const zoom = useRadioStore((s) => s.zoom);
  // Re-renders when the canvas's layout size changes so the HiDPI
  // backing buffer stays matched to the CSS size (keeps the bar,
  // caps, and bandwidth label crisp after any resize).
  const [resizeTick, setResizeTick] = useState(0);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ro = new ResizeObserver(() => setResizeTick((t) => t + 1));
    ro.observe(canvas);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const cssWidth = canvas.clientWidth;
    if (cssWidth <= 0) return;
    const cssHeight = HEIGHT_PX;
    const targetW = Math.max(1, Math.round(cssWidth * dpr));
    const targetH = Math.max(1, Math.round(cssHeight * dpr));
    if (canvas.width !== targetW) canvas.width = targetW;
    if (canvas.height !== targetH) canvas.height = targetH;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssWidth, cssHeight);

    const spanHz = sampleRateHz / zoom;
    if (!Number.isFinite(spanHz) || spanHz <= 0) return;

    const centerX = cssWidth / 2;
    const halfBwPx = Math.max(1, (bandwidthHz / spanHz) * cssWidth * 0.5);
    const rawLeftX = centerX - halfBwPx;
    const rawRightX = centerX + halfBwPx;
    const leftX = Math.max(0, rawLeftX);
    const rightX = Math.min(cssWidth, rawRightX);

    // Soft glow column under the passband — a vertical gradient that
    // fades out toward the bottom. Gives a sense of the filter
    // "enveloping" the signal without competing with the waterfall.
    const gradient = ctx.createLinearGradient(0, 0, 0, cssHeight);
    gradient.addColorStop(0, GLOW_TOP);
    gradient.addColorStop(1, GLOW_BOTTOM);
    ctx.fillStyle = gradient;
    ctx.fillRect(leftX, 0, Math.max(0, rightX - leftX), cssHeight);

    // Bracket bar across the passband.
    if (rightX > leftX) {
      ctx.fillStyle = BAR_COLOR;
      ctx.fillRect(leftX, BAR_Y - BAR_THICKNESS / 2, rightX - leftX, BAR_THICKNESS);
    }

    // End caps at each shoulder (skip if the shoulder is clipped
    // off-canvas at high zoom).
    ctx.strokeStyle = ACCENT;
    ctx.lineWidth = 1;
    const drawCap = (x: number) => {
      if (x < 0 || x > cssWidth) return;
      const xs = Math.round(x) + 0.5;
      ctx.beginPath();
      ctx.moveTo(xs, BAR_Y - CAP_HEIGHT / 2);
      ctx.lineTo(xs, BAR_Y + CAP_HEIGHT / 2);
      ctx.stroke();
    };
    drawCap(rawLeftX);
    drawCap(rawRightX);

    // Center pointer — thin white vertical line + cyan diamond cap.
    const xc = Math.round(centerX) + 0.5;
    ctx.strokeStyle = CENTER_LINE_COLOR;
    ctx.beginPath();
    ctx.moveTo(xc, 0);
    ctx.lineTo(xc, cssHeight);
    ctx.stroke();

    // Diamond marker sitting on the bar, reinforcing the tuned-center
    // intersection. Drawn after the bar so it renders on top.
    ctx.save();
    ctx.translate(xc, BAR_Y);
    ctx.rotate(Math.PI / 4);
    ctx.fillStyle = ACCENT;
    ctx.fillRect(-DIAMOND_HALF, -DIAMOND_HALF, DIAMOND_HALF * 2, DIAMOND_HALF * 2);
    ctx.restore();

    // Bandwidth label above the bar — only if the passband is wide
    // enough to hold it without crowding the caps.
    if (halfBwPx >= LABEL_MIN_HALF_PX) {
      ctx.font = "9px 'JetBrains Mono', ui-monospace, monospace";
      ctx.textAlign = "center";
      ctx.textBaseline = "alphabetic";
      ctx.fillStyle = LABEL_COLOR;
      ctx.fillText(formatBandwidth(bandwidthHz), centerX, LABEL_BASELINE);
    }
  }, [bandwidthHz, sampleRateHz, zoom, resizeTick]);

  return <canvas ref={canvasRef} className="filter-band-marker" aria-hidden="true" />;
};

export default FilterBandMarker;
