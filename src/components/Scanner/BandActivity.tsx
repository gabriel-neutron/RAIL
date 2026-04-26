import { useCallback, useEffect, useMemo, useRef } from "react";
import { buildColormapLut } from "../Waterfall/colormap";
import type { ScanResult } from "../../store/scanner";

const SNR_FLOOR = 0;
const SNR_CEIL = 40;

const LUT = buildColormapLut(256);

const snrToLutIndex = (snr: number): number => {
  const t = (snr - SNR_FLOOR) / (SNR_CEIL - SNR_FLOOR);
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
    const m = new Map<number, { signalAvgDb: number; noiseFloorDb: number }>();
    for (const r of results) {
      m.set(r.frequencyHz, { signalAvgDb: r.signalAvgDb, noiseFloorDb: r.noiseFloorDb });
    }
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

    // Fill colour bars — brightness encodes SNR (0 = noise floor, 40 dB = peak).
    for (let i = 0; i < n; i += 1) {
      const freq = frequenciesHz[i];
      const data = resultsMap.get(freq);
      const x = Math.round((i / n) * w);
      const nextX = Math.round(((i + 1) / n) * w);
      const segW = Math.max(1, nextX - x);

      if (data === undefined) {
        ctx.fillStyle = "#141a22";
      } else {
        const snr = data.signalAvgDb - data.noiseFloorDb;
        const idx = snrToLutIndex(snr);
        const r = LUT[idx * 3];
        const g = LUT[idx * 3 + 1];
        const b = LUT[idx * 3 + 2];
        ctx.fillStyle = `rgb(${r},${g},${b})`;
      }
      ctx.fillRect(x, 0, segW, h);
    }

    // Cyan marker lines for steps whose SNR exceeds the threshold.
    ctx.lineWidth = 1;
    for (let i = 0; i < n; i += 1) {
      const freq = frequenciesHz[i];
      const data = resultsMap.get(freq);
      if (data !== undefined && (data.signalAvgDb - data.noiseFloorDb) > threshold) {
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
