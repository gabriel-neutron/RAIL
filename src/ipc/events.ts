// The single subscribe seam for named Tauri events (Rust → React).
//
// Wire names and payload shapes are generated from shared/ipc_events.json
// into ./generated/events — import those directly, this module deliberately
// does not re-export them.
//
// Waterfall frames travel on a per-session Channel (see ipc/commands.ts),
// not on the event bus.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { IpcEventName, IpcEventPayloads } from "./generated/events";

export const subscribeIpcEvent = <K extends IpcEventName>(
  name: K,
  handler: (payload: IpcEventPayloads[K]) => void,
): Promise<UnlistenFn> =>
  listen<IpcEventPayloads[K]>(name, (evt) => handler(evt.payload));
