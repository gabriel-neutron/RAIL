// Scientific-instrument frequency scale drawn between the spectrum
// and the filter marker. Two-tier ticks: a 1-2-5 × 10^n major step
// targeting ~12 labeled divisions, plus 5 unlabeled minor ticks
// between each major. The major tick nearest the tuned frequency
// gets a phosphor-cyan highlight so the axis reads as "anchored" to
// the tuned center, matching the filter marker's accent.
//
// Read-only view of `frequencyHz`, `sampleRateHz`, `zoom` — redraws
// on those changes only, never per waterfall frame.

import { useEffect, useRef } from "react";

import { useResizeTick } from "../../hooks/useResizeTick";
import { useRadioStore } from "../../store/radio";
import { prepareCanvas2d } from "../../viewport/canvasSizing";
import { formatHz } from "../../viewport/formatHz";
import { createSpectrumViewport } from "../../viewport/spectrumViewport";

const HEIGHT_PX = 24;
const TARGET_TICKS = 12;
const MINOR_SUBDIVISIONS = 5;

const LABEL_COLOR = "#9aa7b5";
const LABEL_CENTER_COLOR = "#e7ebf1";
const TICK_MAJOR_COLOR = "#6b7785";
const TICK_MINOR_COLOR = "#2a3442";
const BASELINE_COLOR = "#1a2230";
const BASELINE_GLOW = "rgba(126, 231, 255, 0.12)";
const ACCENT_TUNED = "#7ee7ff";

/// Snap `raw` to the nearest {1, 2, 5} × 10^n step so tick labels
/// land on round numbers.
const niceStep = (raw: number): number => {
  if (raw <= 0 || !Number.isFinite(raw)) return 1;
  const exponent = Math.floor(Math.log10(raw));
  const pow10 = Math.pow(10, exponent);
  const mantissa = raw / pow10;
  let nice: number;
  if (mantissa < 1.5) nice = 1;
  else if (mantissa < 3) nice = 2;
  else if (mantissa < 7) nice = 5;
  else nice = 10;
  return nice * pow10;
};

export const FrequencyAxis = () => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const frequencyHz = useRadioStore((s) => s.frequencyHz);
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

    const view = createSpectrumViewport({
      centerHz: frequencyHz,
      sampleRateHz,
      zoom,
      cssWidthPx: cssWidth,
    });
    if (view === null) return;
    const { minHz, maxHz, hzToX } = view;
    const step = niceStep(view.spanHz / TARGET_TICKS);
    const minorStep = step / MINOR_SUBDIVISIONS;

    // Etched rule: thin dark baseline + a soft phosphor glow two
    // pixels below it. Reads like an engraved lab-instrument scale.
    ctx.strokeStyle = BASELINE_COLOR;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, 0.5);
    ctx.lineTo(cssWidth, 0.5);
    ctx.stroke();
    ctx.strokeStyle = BASELINE_GLOW;
    ctx.beginPath();
    ctx.moveTo(0, 2.5);
    ctx.lineTo(cssWidth, 2.5);
    ctx.stroke();

    // Minor ticks — drawn first so majors render on top. Skip
    // positions that coincide with a major tick.
    const firstMinor = Math.ceil(minHz / minorStep) * minorStep;
    ctx.strokeStyle = TICK_MINOR_COLOR;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let hz = firstMinor; hz <= maxHz + minorStep * 1e-6; hz += minorStep) {
      const ratio = hz / step;
      if (Math.abs(ratio - Math.round(ratio)) < 1e-6) continue;
      const xs = Math.round(hzToX(hz)) + 0.5;
      ctx.moveTo(xs, 0);
      ctx.lineTo(xs, 3);
    }
    ctx.stroke();

    // Major tick whose value is closest to the tuned frequency — gets
    // the cyan highlight and the bold label. May not exist if the
    // tuned freq lies exactly on the edge of the span.
    const centerTickHz = Math.round(frequencyHz / step) * step;

    const firstMajor = Math.ceil(minHz / step) * step;
    const majorHzList: number[] = [];
    for (let hz = firstMajor; hz <= maxHz + step * 1e-6; hz += step) {
      majorHzList.push(hz);
    }

    // Pick a label stride (1, 2, 4, …) so adjacent rendered labels
    // don't collide. Measured label width + 10 px padding is the
    // minimum spacing required; stride = ceil(needed / available).
    ctx.font = "10px 'JetBrains Mono', ui-monospace, monospace";
    const labels = majorHzList.map((hz) => formatHz(hz, { stepHz: step }));
    let maxLabelWidth = 0;
    for (const l of labels) {
      const w = ctx.measureText(l).width;
      if (w > maxLabelWidth) maxLabelWidth = w;
    }
    const majorSpacingPx = Math.max(1, view.hzWidthToPx(step));
    const labelStride = Math.max(
      1,
      Math.ceil((maxLabelWidth + 10) / majorSpacingPx),
    );

    const majorY = 6;
    const centerMajorY = 9;
    const labelY = cssHeight - 4;
    ctx.textBaseline = "alphabetic";
    ctx.textAlign = "center";

    for (let i = 0; i < majorHzList.length; i += 1) {
      const hz = majorHzList[i];
      const x = hzToX(hz);
      const xs = Math.round(x) + 0.5;
      const isCenter = Math.abs(hz - centerTickHz) < step * 1e-6;

      if (isCenter) {
        ctx.strokeStyle = ACCENT_TUNED;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(xs, 0);
        ctx.lineTo(xs, centerMajorY);
        ctx.stroke();
        ctx.lineWidth = 1;
      } else {
        ctx.strokeStyle = TICK_MAJOR_COLOR;
        ctx.beginPath();
        ctx.moveTo(xs, 0);
        ctx.lineTo(xs, majorY);
        ctx.stroke();
      }

      // Always label the tuned-center tick so the readout never
      // disappears when stride skips it.
      const showLabel = i % labelStride === 0 || isCenter;
      if (!showLabel) continue;

      const halfLabel = maxLabelWidth / 2 + 2;
      const labelX = Math.max(halfLabel, Math.min(cssWidth - halfLabel, x));
      if (isCenter) {
        ctx.fillStyle = LABEL_CENTER_COLOR;
        ctx.font = "bold 10px 'JetBrains Mono', ui-monospace, monospace";
      } else {
        ctx.fillStyle = LABEL_COLOR;
        ctx.font = "10px 'JetBrains Mono', ui-monospace, monospace";
      }
      ctx.fillText(labels[i], labelX, labelY);
    }

    // DC spike from the fs/4 LO offset technique (see docs/DSP.md §1–3).
    // The hardware LO is parked at frequencyHz − sampleRateHz/4; the digital
    // fs/4 mixer shifts the signal of interest to canvas center while pushing
    // the DC spike to frequencyHz − sampleRateHz/4 in real Hz. That quarter
    // is of the FULL sample rate, not of the zoomed span — it reads
    // `sampleRateHz` directly and must not become `view.spanHz / 4`.
    const dcSpikeHz = frequencyHz - sampleRateHz / 4;
    if (dcSpikeHz >= minHz && dcSpikeHz <= maxHz) {
      const xDc = hzToX(dcSpikeHz);
      const xsDc = Math.round(xDc) + 0.5;
      ctx.save();
      ctx.strokeStyle = "rgba(255, 180, 60, 0.7)";
      ctx.lineWidth = 1;
      ctx.setLineDash([2, 3]);
      ctx.beginPath();
      ctx.moveTo(xsDc, 0);
      ctx.lineTo(xsDc, cssHeight);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.restore();
      ctx.fillStyle = "rgba(255, 180, 60, 0.9)";
      ctx.font = "9px 'JetBrains Mono', ui-monospace, monospace";
      ctx.textAlign = "left";
      ctx.fillText("DC", Math.min(xDc + 2, cssWidth - 18), labelY);
    }

    // Frame the band with a 1 px bottom border so the axis reads as
    // a discrete strip between spectrum and filter marker.
    ctx.strokeStyle = BASELINE_COLOR;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, cssHeight - 0.5);
    ctx.lineTo(cssWidth, cssHeight - 0.5);
    ctx.stroke();
  }, [frequencyHz, sampleRateHz, zoom, resizeTick]);

  return <canvas ref={canvasRef} className="freq-axis-canvas" aria-hidden="true" />;
};

export default FrequencyAxis;
