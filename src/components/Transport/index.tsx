import { useEffect, useMemo, useRef, useState } from "react";

import { useReplayStore } from "../../store/replay";

const pad2 = (n: number) => n.toString().padStart(2, "0");
const pad3 = (n: number) => n.toString().padStart(3, "0");

const formatTime = (ms: number): string => {
  const clamped = Math.max(0, Math.floor(ms));
  const minutes = Math.floor(clamped / 60_000);
  const seconds = Math.floor((clamped % 60_000) / 1_000);
  const millis = clamped % 1_000;
  return `${pad2(minutes)}:${pad2(seconds)}.${pad3(millis)}`;
};

const formatMhz = (hz: number): string => (hz / 1_000_000).toFixed(3);

export const Transport = () => {
  const active = useReplayStore((s) => s.active);
  const playing = useReplayStore((s) => s.playing);
  const positionMs = useReplayStore((s) => s.positionMs);
  const info = useReplayStore((s) => s.info);
  const togglePlay = useReplayStore((s) => s.togglePlay);
  const seek = useReplayStore((s) => s.seek);
  const close = useReplayStore((s) => s.close);

  const clampedPosition = useMemo(() => {
    if (!info) return 0;
    return Math.min(Math.max(0, positionMs), info.durationMs);
  }, [positionMs, info]);

  // Local draft for the slider. While the user is dragging, the input
  // fires a change event on every pixel of travel; committing each one
  // to the backend retriggers the 360-row waterfall prefill and pegs
  // the DSP thread. Instead we keep the scrub purely local and only
  // call `seek()` when the user releases (pointer/key/blur), so one
  // drag = one seek.
  const [draftMs, setDraftMs] = useState<number | null>(null);
  const draftRef = useRef<number | null>(null);
  draftRef.current = draftMs;

  useEffect(() => {
    const commit = () => {
      const d = draftRef.current;
      if (d === null) return;
      setDraftMs(null);
      void seek(d);
    };
    window.addEventListener("pointerup", commit);
    window.addEventListener("pointercancel", commit);
    window.addEventListener("keyup", commit);
    return () => {
      window.removeEventListener("pointerup", commit);
      window.removeEventListener("pointercancel", commit);
      window.removeEventListener("keyup", commit);
    };
  }, [seek]);

  if (!active || !info) return null;

  const displayMs = draftMs ?? clampedPosition;

  return (
    <section className="transport-bar" aria-label="IQ replay transport">
      <button
        type="button"
        className="transport-btn"
        onClick={() => void togglePlay()}
        aria-label={playing ? "Pause" : "Play"}
      >
        {playing ? "Pause" : "Play"}
      </button>
      <span className="transport-time">{formatTime(displayMs)}</span>
      <input
        type="range"
        className="transport-slider"
        min={0}
        max={info.durationMs}
        step={10}
        value={displayMs}
        onChange={(e) => setDraftMs(Number(e.target.value))}
        aria-label="Seek"
      />
      <span className="transport-time">{formatTime(info.durationMs)}</span>
      <span className="transport-meta">
        {formatMhz(info.centerFrequencyHz)} MHz · {info.demodMode} ·{" "}
        {(info.sampleRateHz / 1_000_000).toFixed(3)} Msps
      </span>
      <button
        type="button"
        className="transport-btn transport-close"
        onClick={() => void close()}
      >
        Close
      </button>
    </section>
  );
};

export default Transport;
