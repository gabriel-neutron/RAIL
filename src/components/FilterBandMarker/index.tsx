// Scientific-instrument filter-passband indicator. Replaces the
// trapezoid skirt with a horizontal bracket (bar + end caps),
// a soft phosphor-cyan glow column across the passband, and a thin
// center-pointer with a diamond cap marking the tuned bin. The
// optional bandwidth label prints above the bracket when there's
// enough horizontal room.
//
// Read-only view of `bandwidthHz`, `sampleRateHz`, `zoom` — redraws
// only when one of those changes, not per waterfall frame.

import { useEffect, useRef } from "react";

import { useResizeTick } from "../../hooks/useResizeTick";
import { useRadioStore } from "../../store/radio";
import { prepareCanvas2d } from "../../viewport/canvasSizing";
import { formatHz } from "../../viewport/formatHz";
import { createSpectrumViewport } from "../../viewport/spectrumViewport";

const HEIGHT_PX = 26;
const ACCENT = "#ffb229";
const BAR_COLOR = "rgba(255, 178, 41, 0.45)";
const GLOW_TOP = "rgba(255, 178, 41, 0.08)";
const GLOW_BOTTOM = "rgba(255, 178, 41, 0)";
const CENTER_LINE_COLOR = "rgba(255, 244, 221, 0.8)";
const LABEL_COLOR = "rgba(255, 178, 41, 0.85)";

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

export const FilterBandMarker = () => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const bandwidthHz = useRadioStore((s) => s.bandwidthHz);
  const sampleRateHz = useRadioStore((s) => s.sampleRateHz);
  const zoom = useRadioStore((s) => s.zoom);
  const resizeTick = useResizeTick(canvasRef);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const sized = prepareCanvas2d(canvas, HEIGHT_PX, window.devicePixelRatio || 1);
    if (!sized) return;
    const { ctx, cssWidthPx: cssWidth } = sized;
    const cssHeight = HEIGHT_PX;
    ctx.clearRect(0, 0, cssWidth, cssHeight);

    // Only the RELATIVE conversion is used here, so the centre is irrelevant
    // and passed as 0 — this component never subscribes to `frequencyHz`.
    // The marker is drawn symmetrically about canvas centre rather than via
    // `hzToX(frequencyHz ± bw/2)`: the tuned centre sits at canvas centre by
    // construction, and reading it here would redraw the bracket on every
    // debounced retune during a pan for numerically identical output.
    const view = createSpectrumViewport({
      centerHz: 0,
      sampleRateHz,
      zoom,
      cssWidthPx: cssWidth,
    });
    if (view === null) return;

    const centerX = cssWidth / 2;
    const halfBwPx = Math.max(1, view.hzWidthToPx(bandwidthHz) / 2);
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
      ctx.fillText(formatHz(bandwidthHz), centerX, LABEL_BASELINE);
    }
  }, [bandwidthHz, sampleRateHz, zoom, resizeTick]);

  return <canvas ref={canvasRef} className="filter-band-marker" aria-hidden="true" />;
};

export default FilterBandMarker;
