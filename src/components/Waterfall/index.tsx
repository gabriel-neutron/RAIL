import { useEffect, useMemo, useRef } from "react";

import { useWaterfall } from "../../hooks/useWaterfall";
import { useRadioStore } from "../../store/radio";
import { buildColormapLut } from "./colormap";

const DEFAULT_FFT_SIZE = 2048;
const DISPLAY_HEIGHT = 400;
const DB_FLOOR = -100;
const DB_PEAK = -20;

type WaterfallProps = {
  enabled?: boolean;
  onAudio?: (frame: Float32Array) => void;
};

export const Waterfall = ({ enabled = true, onAudio }: WaterfallProps) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rowImageRef = useRef<ImageData | null>(null);
  const lut = useMemo(() => buildColormapLut(256), []);

  const { session, error } = useWaterfall({
    enabled,
    onAudio,
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

  // Pixel X → frequency mapping: DC lands at bin N/2 after `fft_shift`
  // (docs/DSP.md §1–3; verified by `dc_input_peaks_at_center_after_shift`).
  // So pixel X over the canvas span maps linearly to [-fs/2, +fs/2].
  const handleCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const store = useRadioStore.getState();
    if (!store.streaming) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0) return;
    const xNorm = (e.clientX - rect.left) / rect.width;
    const offsetHz = (xNorm - 0.5) * store.sampleRateHz;
    store.setFrequency(store.frequencyHz + offsetHz);
  };

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
        onClick={handleCanvasClick}
      />
    </section>
  );
};

export default Waterfall;
