// Typed wrappers for Tauri events (Rust → React).
// Event names and payload shapes defined in docs/ARCHITECTURE.md §3.
// Waterfall frames travel on a per-session Channel (see ipc/commands.ts),
// not on the event bus.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const EVENT_DEVICE_STATUS = "device-status";

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
