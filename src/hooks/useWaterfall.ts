// Waterfall + audio session lifecycle hook.
//
// Opens a pair of binary `Channel<ArrayBuffer>`s (one for spectrum
// frames, one for f32 PCM audio) and calls either `start_stream`
// (live RTL-SDR) or `start_replay` (SigMF file) depending on whether
// `useReplayStore.active` is true. Drives a rAF drain of the latest
// waterfall frame into `onFrame`, forwards audio chunks synchronously
// to `onAudio`, and tears the session down on unmount or when the
// live/replay source flips.
//
// Frequency changes during a live session go through the store's
// debounced `retune` path (`store/radio.ts`); replay sessions ignore
// retune calls server-side.
//
// See docs/ARCHITECTURE.md §3 and docs/DSP.md §3–4.

import { Channel } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";

import {
  startReplay,
  startStream,
  stopStream,
  type RailError,
  type StartStreamReply,
} from "../ipc/commands";
import { useRadioStore } from "../store/radio";
import { useReplayStore } from "../store/replay";

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

  // Re-key the effect when the source flips between live and replay
  // so a running live stream tears down before `start_replay` fires
  // (the backend only allows one session at a time).
  const replayActive = useReplayStore((s) => s.active);
  const replayDataPath = useReplayStore((s) => s.info?.dataPath ?? null);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    let cancelled = false;
    let rafId: number | null = null;
    // Queue every frame the backend sends so a burst (e.g. the 360-row
    // waterfall prefill emitted by `src-tauri/src/replay.rs` on seek)
    // paints every row. During live streaming the backend already rate-
    // limits to ~25 fps so the queue sits at 0–1.
    const pending: Float32Array[] = [];

    const waterfallChannel = new Channel<ArrayBuffer>();
    waterfallChannel.onmessage = (buffer) => {
      pending.push(new Float32Array(buffer));
    };

    const audioChannel = new Channel<ArrayBuffer>();
    audioChannel.onmessage = (buffer) => {
      const handler = onAudioRef.current;
      if (handler) handler(new Float32Array(buffer));
    };

    const drain = () => {
      if (cancelled) return;
      const handler = onFrameRef.current;
      // Cap per-tick work so a pathological backlog can't stall the
      // main thread. 360 matches the waterfall canvas height, which
      // is also the max prefill burst size.
      let budget = 360;
      while (pending.length > 0 && budget > 0) {
        handler(pending.shift()!);
        budget -= 1;
      }
      rafId = window.requestAnimationFrame(drain);
    };

    const radio = useRadioStore.getState();

    (async () => {
      await stopStream().catch(() => undefined);
      if (cancelled) return;
      try {
        let reply: WaterfallSession;
        if (replayActive && replayDataPath) {
          const replyRaw = await startReplay(
            replayDataPath,
            waterfallChannel,
            audioChannel,
          );
          reply = {
            fftSize: replyRaw.fftSize,
            sampleRateHz: replyRaw.sampleRateHz,
            frequencyHz: replyRaw.frequencyHz,
            availableGainsTenthsDb: [],
            audioSampleRateHz: replyRaw.audioSampleRateHz,
            audioChunkSamples: replyRaw.audioChunkSamples,
          };
          // Replay sessions are pinned to the capture's center freq;
          // `setFrequency` is guarded against retune during replay, so
          // we write straight to the store instead.
          useRadioStore.setState({ frequencyHz: replyRaw.frequencyHz });
          if (replyRaw.info.demodMode === "FM" || replyRaw.info.demodMode === "AM") {
            radio.setMode(replyRaw.info.demodMode);
          }
          if (replyRaw.info.filterBandwidthHz > 0) {
            radio.setBandwidth(replyRaw.info.filterBandwidthHz);
          }
        } else {
          reply = await startStream(
            { frequencyHz: radio.frequencyHz },
            waterfallChannel,
            audioChannel,
          );
          radio.setAvailableGains(reply.availableGainsTenthsDb);
        }
        if (cancelled) {
          await stopStream().catch(() => undefined);
          return;
        }
        setSession(reply);
        setError(null);
        radio.setSampleRate(reply.sampleRateHz);
        radio.setStreaming(true);
        rafId = window.requestAnimationFrame(drain);
      } catch (err) {
        if (!cancelled) {
          setError(formatError(err));
          setSession(null);
          radio.setStreaming(false);
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
  }, [enabled, replayActive, replayDataPath]);

  return { session, error };
};
