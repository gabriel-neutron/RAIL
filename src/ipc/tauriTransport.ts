// The one module in `src/` that statically imports the Tauri runtime.
// Keep it that way: anything else importing `@tauri-apps` drags the
// runtime into store and command tests. See `docs/CONVENTIONS.md` §2.

import { Channel, invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

import type {
  IpcChannel,
  IpcTransport,
  OpenPathOptions,
  SavePathOptions,
} from "./transport";

export const tauriTransport: IpcTransport = {
  invoke: <T,>(command: string, payload?: Record<string, unknown>): Promise<T> =>
    invoke<T>(command, payload),

  createChannel: <T,>(onMessage: (message: T) => void): IpcChannel<T> =>
    new Channel<T>(onMessage),

  pickOpenPath: async (options: OpenPathOptions): Promise<string | null> => {
    // `multiple: false` still types as `string | string[] | null`;
    // collapse it here so callers only ever see one path.
    const picked = await open({ multiple: false, filters: options.filters });
    if (picked === null || Array.isArray(picked)) return null;
    return picked;
  },

  pickSavePath: (options: SavePathOptions): Promise<string | null> =>
    save({ filters: options.filters, defaultPath: options.defaultPath }),
};
