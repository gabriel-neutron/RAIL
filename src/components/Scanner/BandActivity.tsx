import { useCallback, useEffect, useMemo, useRef } from "react";
import { buildColormapLut } from "../Waterfall/colormap";
import type { ScanResult } from "../../store/scanner";

const DB_FLOOR = -100;
const DB_CEIL = -20;
const DB_SPAN = DB_CEIL - DB_FLOOR;

const LUT = buildColormapLut(256);

const dbfsToLutIndex = (dbfs: number): number => {
  const t = (dbfs - DB_FLOOR) / DB_SPAN;
  return Math.max(0, Math.min(255, Math.round(t * 255)));
};

type Props = {
  frequenciesHz: number[];
  results: ScanResult[];
  threshold: number;
  selectedFrequencyHz?: number;
  onTune: (frequencyHz: number) => void;
};

export const BandActivity = ({
  frequenciesHz,
  results,
  threshold,
  selectedFrequencyHz,
  onTune,
}: Props) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const resultsMap = useMemo(() => {
    const m = new Map<number, number>();
    for (const r of results) m.set(r.frequencyHz, r.peakDbfs);
    return m;
  }, [results]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const w = canvas.width;
    const h = canvas.height;
    const n = frequenciesHz.length;

    ctx.clearRect(0, 0, w, h);
    if (n === 0) return;

    // Fill colour bars.
    for (let i = 0; i < n; i += 1) {
      const freq = frequenciesHz[i];
      const dbfs = resultsMap.get(freq);
      const x = Math.round((i / n) * w);
      const nextX = Math.round(((i + 1) / n) * w);
      const segW = Math.max(1, nextX - x);

      if (dbfs === undefined) {
        ctx.fillStyle = "#141a22";
      } else {
        const idx = dbfsToLutIndex(dbfs);
        const r = LUT[idx * 3];
        const g = LUT[idx * 3 + 1];
        const b = LUT[idx * 3 + 2];
        ctx.fillStyle = `rgb(${r},${g},${b})`;
      }
      ctx.fillRect(x, 0, segW, h);
    }

    // Cyan marker lines for peaks above threshold.
    ctx.lineWidth = 1;
    for (let i = 0; i < n; i += 1) {
      const freq = frequenciesHz[i];
      const dbfs = resultsMap.get(freq);
      if (dbfs !== undefined && dbfs > threshold) {
        const x = Math.round(((i + 0.5) / n) * w);
        // White for the currently selected signal, cyan for the rest.
        ctx.strokeStyle = freq === selectedFrequencyHz ? "#ffffff" : "#7ee7ff";
        ctx.beginPath();
        ctx.moveTo(x + 0.5, 0);
        ctx.lineTo(x + 0.5, h);
        ctx.stroke();
      }
    }
  }, [frequenciesHz, resultsMap, threshold, selectedFrequencyHz]);

  const handleClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      if (!canvas || frequenciesHz.length === 0) return;
      const rect = canvas.getBoundingClientRect();
      const ratio = (e.clientX - rect.left) / rect.width;
      const idx = Math.max(
        0,
        Math.min(frequenciesHz.length - 1, Math.round(ratio * (frequenciesHz.length - 1))),
      );
      onTune(frequenciesHz[idx]);
    },
    [frequenciesHz, onTune],
  );

  return (
    <canvas
      ref={canvasRef}
      className="band-activity-canvas"
      width={512}
      height={32}
      onClick={handleClick}
      title="Click to tune"
    />
  );
};

export default BandActivity;
