import { useEffect, useMemo, useRef } from "react";

import { useWaterfall } from "../../hooks/useWaterfall";
import { buildColormapLut } from "./colormap";

const DEFAULT_FFT_SIZE = 2048;
const DISPLAY_HEIGHT = 400;
const DB_FLOOR = -100;
const DB_PEAK = -20;

type WaterfallProps = {
  frequencyHz: number;
  enabled?: boolean;
};

export const Waterfall = ({ frequencyHz, enabled = true }: WaterfallProps) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rowImageRef = useRef<ImageData | null>(null);
  const lut = useMemo(() => buildColormapLut(256), []);

  const { session, error } = useWaterfall({
    frequencyHz,
    enabled,
    onFrame: (frame) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const ctx = canvas.getContext("2d", { alpha: false });
      if (!ctx) return;

      if (canvas.width !== frame.length) {
        canvas.width = frame.length;
      }
      if (canvas.height !== DISPLAY_HEIGHT) {
        canvas.height = DISPLAY_HEIGHT;
      }

      if (
        rowImageRef.current === null ||
        rowImageRef.current.width !== frame.length
      ) {
        rowImageRef.current = ctx.createImageData(frame.length, 1);
      }

      const row = rowImageRef.current;
      const pixels = row.data;
      const span = DB_PEAK - DB_FLOOR;
      const lutEntries = lut.length / 3;
      for (let i = 0; i < frame.length; i += 1) {
        const normalized = Math.max(
          0,
          Math.min(1, (frame[i] - DB_FLOOR) / span),
        );
        const lutIdx = (normalized * (lutEntries - 1)) | 0;
        const offset = lutIdx * 3;
        const out = i * 4;
        pixels[out] = lut[offset];
        pixels[out + 1] = lut[offset + 1];
        pixels[out + 2] = lut[offset + 2];
        pixels[out + 3] = 255;
      }

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
    },
  });

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) return;
    ctx.fillStyle = "#07090c";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
  }, [session?.fftSize]);

  return (
    <section className="waterfall">
      <div className="waterfall-status">
        {error && <span className="waterfall-error">stream error: {error}</span>}
        {!error && session === null && (
          <span className="waterfall-pending">opening stream…</span>
        )}
        {!error && session && (
          <span className="waterfall-ok">
            fs={(session.sampleRateHz / 1e6).toFixed(3)} MHz · N={session.fftSize}
          </span>
        )}
      </div>
      <canvas
        ref={canvasRef}
        className="waterfall-canvas"
        width={DEFAULT_FFT_SIZE}
        height={DISPLAY_HEIGHT}
      />
    </section>
  );
};

export default Waterfall;
