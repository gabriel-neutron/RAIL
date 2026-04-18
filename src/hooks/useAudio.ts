// Web Audio PCM scheduler: consumes f32 mono samples at a fixed
// sample rate and plays them back through a per-session AudioContext.
//
// Incoming chunks arrive asynchronously from the Rust demod chain; we
// schedule each one on the AudioContext timeline with a small lookahead
// so bursty arrivals don't produce audible gaps. Volume and mute are
// applied via a single GainNode so the signal path stays:
//
//     AudioBufferSourceNode → GainNode → ctx.destination
//
// See docs/ARCHITECTURE.md §5.

import { useCallback, useEffect, useMemo, useRef } from "react";

import { useRadioStore } from "../store/radio";

/// Minimum time between the current playback head and the next
/// scheduled buffer. Larger values tolerate more jitter at the cost
/// of audible latency.
const SCHEDULE_LOOKAHEAD_S = 0.08;

/// If the scheduler falls more than this far behind the playback head
/// we restart the clock — catching up would queue a burst of audio
/// that plays back at real-time and just delays the user experience.
const MAX_DRIFT_S = 0.4;

export type UseAudioOptions = {
  enabled: boolean;
  sampleRateHz: number;
};

export type UseAudioApi = {
  /// Enqueue one PCM chunk for playback.
  enqueue: (frame: Float32Array) => void;
  /// Resume the AudioContext. Browsers suspend a freshly created
  /// context until a user gesture; call this from a click/keydown
  /// handler if `enqueue` is silent on startup.
  resume: () => Promise<void>;
};

export const useAudio = ({
  enabled,
  sampleRateHz,
}: UseAudioOptions): UseAudioApi => {
  const ctxRef = useRef<AudioContext | null>(null);
  const gainRef = useRef<GainNode | null>(null);
  const nextStartRef = useRef<number>(0);

  const volume = useRadioStore((s) => s.volume);
  const muted = useRadioStore((s) => s.muted);

  const getCtx = useCallback((): AudioContext | null => {
    if (!enabled || sampleRateHz <= 0) return null;
    if (ctxRef.current) return ctxRef.current;
    try {
      const ctx = new AudioContext();
      const gain = ctx.createGain();
      gain.gain.value = muted ? 0 : volume;
      gain.connect(ctx.destination);
      ctxRef.current = ctx;
      gainRef.current = gain;
      nextStartRef.current = 0;
      return ctx;
    } catch (err) {
      console.warn("[RAIL] AudioContext creation failed:", err);
      return null;
    }
  }, [enabled, sampleRateHz, muted, volume]);

  useEffect(() => {
    if (gainRef.current) {
      gainRef.current.gain.value = muted ? 0 : volume;
    }
  }, [muted, volume]);

  useEffect(() => {
    if (enabled) return;
    const ctx = ctxRef.current;
    ctxRef.current = null;
    gainRef.current = null;
    nextStartRef.current = 0;
    if (ctx) {
      ctx.close().catch((err) => {
        console.warn("[RAIL] AudioContext close failed:", err);
      });
    }
  }, [enabled]);

  useEffect(
    () => () => {
      const ctx = ctxRef.current;
      ctxRef.current = null;
      gainRef.current = null;
      if (ctx) {
        ctx.close().catch(() => undefined);
      }
    },
    [],
  );

  const enqueue = useCallback(
    (frame: Float32Array) => {
      if (frame.length === 0) return;
      const ctx = getCtx();
      const gain = gainRef.current;
      if (!ctx || !gain) return;

      // Browsers may have auto-suspended the context until a user
      // gesture. A suspended context still accepts scheduled buffers
      // but won't actually play; `resume()` will flush them once a
      // gesture lands.
      if (ctx.state === "suspended") {
        ctx.resume().catch(() => undefined);
      }

      const buffer = ctx.createBuffer(1, frame.length, sampleRateHz);
      // `getChannelData().set()` instead of `copyToChannel(frame)` —
      // the latter requires `Float32Array<ArrayBuffer>` in lib.dom
      // (TS 5.7+), but `frame` comes from a Tauri Channel typed as
      // the more permissive `Float32Array<ArrayBufferLike>`.
      buffer.getChannelData(0).set(frame);

      const source = ctx.createBufferSource();
      source.buffer = buffer;
      source.connect(gain);

      const now = ctx.currentTime;
      let startAt = Math.max(nextStartRef.current, now + SCHEDULE_LOOKAHEAD_S);
      if (startAt - now > MAX_DRIFT_S) {
        // We drifted too far ahead (tab throttled, GC pause, etc).
        // Reset the clock rather than queue a stale burst.
        startAt = now + SCHEDULE_LOOKAHEAD_S;
      }
      source.start(startAt);
      nextStartRef.current = startAt + frame.length / sampleRateHz;
    },
    [getCtx, sampleRateHz],
  );

  const resume = useCallback(async () => {
    const ctx = getCtx();
    if (!ctx) return;
    if (ctx.state === "suspended") {
      await ctx.resume().catch(() => undefined);
    }
  }, [getCtx]);

  return useMemo<UseAudioApi>(
    () => ({ enqueue, resume }),
    [enqueue, resume],
  );
};

export default useAudio;
