// Waterfall lifecycle hook: opens a binary `Channel<ArrayBuffer>`, asks
// Rust to start streaming at the store's current frequency, drives a rAF
// drain of the latest frame into the supplied callback, and tears
// everything down on unmount.
//
// Frequency changes after startup go through the store's debounced
// `retune` path (`store/radio.ts`), not through a restart. The effect
// deps here are `[enabled]` only — tuning never tears the stream down.
//
// See docs/ARCHITECTURE.md §3 and docs/DSP.md §3.

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
}: UseWaterfallOptions): UseWaterfallState => {
  const [session, setSession] = useState<WaterfallSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const onFrameRef = useRef(onFrame);

  useEffect(() => {
    onFrameRef.current = onFrame;
  }, [onFrame]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    let cancelled = false;
    let rafId: number | null = null;
    let latest: Float32Array | null = null;

    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buffer) => {
      latest = new Float32Array(buffer);
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
        const reply = await startStream({ frequencyHz: initialFrequencyHz }, channel);
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
