// The seam between the frontend and whatever is carrying its commands.
// Everything under `src/` talks to this port; only `tauriTransport.ts`
// knows the Tauri runtime exists, so stores stay importable in tests.
// See `docs/ARCHITECTURE.md` (frontend control seam).

/// Native file-picker options, narrowed to what RAIL actually asks for.
/// Structural on purpose — re-exporting the dialog plugin's types here
/// would put `@tauri-apps` back in the import graph of every store.
export type PathFilter = {
  name: string;
  extensions: string[];
};

export type OpenPathOptions = {
  filters?: PathFilter[];
};

export type SavePathOptions = {
  filters?: PathFilter[];
  defaultPath?: string;
};

/// Everything the frontend needs from the host. `invoke` carries the
/// commands, the two pickers carry the native dialogs.
export type IpcTransport = {
  invoke<T>(command: string, payload?: Record<string, unknown>): Promise<T>;
  /// Resolves to `null` when the user cancels.
  pickOpenPath(options: OpenPathOptions): Promise<string | null>;
  /// Resolves to `null` when the user cancels.
  pickSavePath(options: SavePathOptions): Promise<string | null>;
};

let active: IpcTransport | null = null;

/// Install the transport. Called once from the composition root
/// (`src/main.tsx`), and per-test with a mock adapter.
export const setTransport = (next: IpcTransport | null): void => {
  active = next;
};

/// The installed transport. Throws rather than degrading silently: a
/// missing transport is a wiring bug, and a swallowed one would look
/// like a dead radio.
export const transport = (): IpcTransport => {
  if (active === null) {
    throw new Error("[RAIL] IPC transport not configured");
  }
  return active;
};
