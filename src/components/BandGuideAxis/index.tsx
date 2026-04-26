// Canvas row rendering known frequency-band allocations directly on the
// frequency axis scale. Redraws only when frequency/zoom/store state changes —
// never per waterfall frame. See docs/DSP.md for the Hz↔pixel transform.

import { useEffect, useRef, useState } from "react";

import { BAND_ENTRIES, type BandCategory } from "../../data/bands";
import { useBandGuideStore } from "../../store/bandGuide";
import { useRadioStore } from "../../store/radio";

const HEIGHT_PX = 16;
const BAR_FILL_ALPHA = "8c"; // 55 % opacity in hex
const BAR_EDGE_ALPHA = "d9"; // 85 % opacity in hex

export const CATEGORY_COLORS: Record<BandCategory, string> = {
  broadcast: "#3a8ef0",
  aviation:  "#e8a020",
  maritime:  "#20b8c8",
  amateur:   "#7e50e8",
  utility:   "#60a860",
  weather:   "#d06060",
  ism:       "#909090",
};

export const BandGuideAxis = () => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const frequencyHz = useRadioStore((s) => s.frequencyHz);
  const sampleRateHz = useRadioStore((s) => s.sampleRateHz);
  const zoom = useRadioStore((s) => s.zoom);
  const visible = useBandGuideStore((s) => s.visible);
  const activeCategories = useBandGuideStore((s) => s.activeCategories);
  const region = useBandGuideStore((s) => s.region);
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

    const targetW = Math.max(1, Math.round(cssWidth * dpr));
    const targetH = Math.max(1, Math.round(HEIGHT_PX * dpr));
    if (canvas.width !== targetW) canvas.width = targetW;
    if (canvas.height !== targetH) canvas.height = targetH;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssWidth, HEIGHT_PX);

    const spanHz = sampleRateHz / zoom;
    if (!Number.isFinite(spanHz) || spanHz <= 0) return;
    const minHz = frequencyHz - spanHz / 2;
    const maxHz = frequencyHz + spanHz / 2;

    const hzToX = (hz: number) => ((hz - minHz) / spanHz) * cssWidth;

    // Filter to visible, active, and region-matching bands.
    const visible_bands = BAND_ENTRIES.filter(
      (b) =>
        b.maxHz > minHz &&
        b.minHz < maxHz &&
        activeCategories.has(b.category) &&
        (region === "global" ? b.region === "global" : b.region === region || b.region === "global"),
    );

    // Sort widest-first so wide bands get label priority.
    visible_bands.sort((a, b) => (b.maxHz - b.minHz) - (a.maxHz - a.minHz));

    ctx.font = "10px Inter, ui-sans-serif, sans-serif";
    const occupiedRanges: Array<[number, number]> = [];

    for (const band of visible_bands) {
      const x0 = Math.max(0, hzToX(band.minHz));
      const x1 = Math.min(cssWidth, hzToX(band.maxHz));
      if (x1 <= x0) continue;

      const color = CATEGORY_COLORS[band.category];

      // Fill bar.
      ctx.fillStyle = color + BAR_FILL_ALPHA;
      ctx.fillRect(x0, 1, x1 - x0, HEIGHT_PX - 2);

      // Top edge accent.
      ctx.strokeStyle = color + BAR_EDGE_ALPHA;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x0, 1.5);
      ctx.lineTo(x1, 1.5);
      ctx.stroke();

      // Label — try full label, then shortLabel; skip if no room or collision.
      const barWidth = x1 - x0;
      const PADDING = 4;
      let labelText: string | null = null;
      for (const text of [band.label, band.shortLabel]) {
        const textW = ctx.measureText(text).width;
        if (textW + PADDING * 2 <= barWidth) {
          labelText = text;
          break;
        }
      }

      if (labelText !== null) {
        const cx = (x0 + x1) / 2;
        const half = ctx.measureText(labelText).width / 2 + PADDING;
        const lx0 = cx - half;
        const lx1 = cx + half;

        const collides = occupiedRanges.some(([a, b]) => lx1 > a && lx0 < b);
        if (!collides) {
          occupiedRanges.push([lx0, lx1]);
          ctx.fillStyle = "#e7ebf1";
          ctx.textBaseline = "middle";
          ctx.textAlign = "center";
          ctx.fillText(labelText, cx, HEIGHT_PX / 2 + 1);
        }
      }
    }

    // Bottom separator line.
    ctx.strokeStyle = "#1a2230";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, HEIGHT_PX - 0.5);
    ctx.lineTo(cssWidth, HEIGHT_PX - 0.5);
    ctx.stroke();
  }, [frequencyHz, sampleRateHz, zoom, visible, activeCategories, region, resizeTick]);

  if (!visible) return null;

  return (
    <canvas
      ref={canvasRef}
      className="band-guide-axis"
      aria-hidden="true"
    />
  );
};

export default BandGuideAxis;
