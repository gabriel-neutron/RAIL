// Waterfall + audio session lifecycle hook.
//
// Opens a pair of binary `Channel<ArrayBuffer>`s (one for spectrum
// frames, one for f32 PCM audio), calls `start_stream` at the store's
// current frequency, drives a rAF drain of the latest waterfall frame
// into `onFrame`, forwards audio frames to `onAudio` synchronously,
// and tears everything down on unmount.
//
// Frequency changes after startup go through the store's debounced
// `retune` path (`store/radio.ts`), not through a restart. The effect
// deps here are `[enabled]` only — tuning never tears the stream down.
//
// See docs/ARCHITECTURE.md §3 and docs/DSP.md §3–4.

import { Channel } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";

import {
  startStream,
  stopStream,
  type RailError,
  type StartStreamReply,
} from "../ipc/commands";
import { useRadioStore } from "../store/radio";

export type WaterfallSession = StartStreamReply;

export type UseWaterfallOptions = {
  enabled?: boolean;
  onFrame: (frame: Float32Array) => void;
  onAudio?: (frame: Float32Array) => void;
};

export type UseWaterfallState = {
  session: WaterfallSession | null;
  error: string | null;
};

const isRailError = (value: unknown): value is RailError => {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    typeof (value as { kind: unknown }).kind === "string"
  );
};

const formatError = (err: unknown): string => {
  if (isRailError(err)) {
    return err.message ? `${err.kind}: ${err.message}` : err.kind;
  }
  return String(err);
};

export const useWaterfall = ({
  enabled = true,
  onFrame,
  onAudio,
}: UseWaterfallOptions): UseWaterfallState => {
  const [session, setSession] = useState<WaterfallSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const onFrameRef = useRef(onFrame);
  const onAudioRef = useRef(onAudio);

  useEffect(() => {
    onFrameRef.current = onFrame;
  }, [onFrame]);

  useEffect(() => {
    onAudioRef.current = onAudio;
  }, [onAudio]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    let cancelled = false;
    let rafId: number | null = null;
    let latest: Float32Array | null = null;

    const waterfallChannel = new Channel<ArrayBuffer>();
    waterfallChannel.onmessage = (buffer) => {
      latest = new Float32Array(buffer);
    };

    const audioChannel = new Channel<ArrayBuffer>();
    audioChannel.onmessage = (buffer) => {
      // Audio is time-critical: deliver every chunk synchronously
      // instead of batching by rAF. Drops here would cause clicks.
      const handler = onAudioRef.current;
      if (handler) handler(new Float32Array(buffer));
    };

    const drain = () => {
      if (cancelled) return;
      if (latest) {
        const frame = latest;
        latest = null;
        onFrameRef.current(frame);
      }
      rafId = window.requestAnimationFrame(drain);
    };

    const store = useRadioStore.getState();
    const initialFrequencyHz = store.frequencyHz;

    (async () => {
      // Drain any previous session before starting a new one. Guards
      // against rapid enable toggles beating the backend's "stream
      // already running" check.
      await stopStream().catch(() => undefined);
      if (cancelled) return;
      try {
        const reply = await startStream(
          { frequencyHz: initialFrequencyHz },
          waterfallChannel,
          audioChannel,
        );
        if (cancelled) {
          await stopStream().catch(() => undefined);
          return;
        }
        setSession(reply);
        setError(null);
        store.setAvailableGains(reply.availableGainsTenthsDb);
        store.setSampleRate(reply.sampleRateHz);
        store.setStreaming(true);
        rafId = window.requestAnimationFrame(drain);
      } catch (err) {
        if (!cancelled) {
          setError(formatError(err));
          setSession(null);
          store.setStreaming(false);
        }
      }
    })();

    return () => {
      cancelled = true;
      if (rafId !== null) {
        window.cancelAnimationFrame(rafId);
      }
      stopStream().catch((err) => {
        console.warn("[RAIL] stopStream failed on teardown:", err);
      });
      setSession(null);
      useRadioStore.getState().setStreaming(false);
    };
  }, [enabled]);

  return { session, error };
};
