// Typed wrappers for Tauri events (Rust → React).
// Event names and payload shapes defined in docs/ARCHITECTURE.md §3.
// Waterfall frames travel on a per-session Channel (see ipc/commands.ts),
// not on the event bus.
//
// String literals: shared/ipc_event_names.json (see scripts/gen-ipc-event-names.mjs).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  EVENT_DEVICE_STATUS,
  EVENT_REPLAY_POSITION,
  EVENT_SIGNAL_LEVEL,
} from "./generated/eventNames";

export { EVENT_DEVICE_STATUS, EVENT_REPLAY_POSITION, EVENT_SIGNAL_LEVEL };

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
