// Narrowing and display for errors crossing the Tauri boundary. Commands
// reject with a serialized `RailError` (see `src-tauri/src/error.rs`), but
// anything can surface as `unknown`, so callers need the guard.

import type { RailError } from "./commands";

export const isRailError = (value: unknown): value is RailError =>
  typeof value === "object" &&
  value !== null &&
  "kind" in value &&
  typeof (value as RailError).kind === "string";

/// Renders a backend error for display. Falls back to `String(err)` for
/// anything that did not come from the Rust side.
export const formatIpcError = (err: unknown): string => {
  if (!isRailError(err)) return String(err);
  return err.message ? `${err.kind}: ${err.message}` : err.kind;
};
