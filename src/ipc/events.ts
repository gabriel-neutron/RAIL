// Typed wrappers for Tauri events (Rust → React).
// Event names and payload shapes defined in docs/ARCHITECTURE.md §3.
// Waterfall frames travel on a per-session Channel (see ipc/commands.ts),
// not on the event bus.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const EVENT_DEVICE_STATUS = "device-status";
export const EVENT_SIGNAL_LEVEL = "signal-level";
export const EVENT_REPLAY_POSITION = "replay-position";

export type DeviceStatusPayload = {
  connected: boolean;
  error?: string;
};

export const subscribeDeviceStatus = (
  handler: (payload: DeviceStatusPayload) => void,
): Promise<UnlistenFn> =>
  listen<DeviceStatusPayload>(EVENT_DEVICE_STATUS, (evt) =>
    handler(evt.payload),
  );

/// Periodic dBFS level + decaying peak-hold. Backend decays peak by
/// ~1 dB per emission at ~25 Hz (see `MIN_LEVEL_EMIT_INTERVAL` in
/// `src-tauri/src/ipc/commands.rs`).
export type SignalLevelPayload = {
  current: number;
  peak: number;
};

export const subscribeSignalLevel = (
  handler: (payload: SignalLevelPayload) => void,
): Promise<UnlistenFn> =>
  listen<SignalLevelPayload>(EVENT_SIGNAL_LEVEL, (evt) =>
    handler(evt.payload),
  );

/// IQ-replay transport position. Backend emits at ~25 Hz from the
/// replay reader thread (see `src-tauri/src/replay.rs`).
export type ReplayPositionPayload = {
  sampleIdx: number;
  positionMs: number;
  totalMs: number;
  playing: boolean;
};

export const subscribeReplayPosition = (
  handler: (payload: ReplayPositionPayload) => void,
): Promise<UnlistenFn> =>
  listen<ReplayPositionPayload>(EVENT_REPLAY_POSITION, (evt) =>
    handler(evt.payload),
  );
