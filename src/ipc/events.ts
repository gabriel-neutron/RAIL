// Typed wrappers for Tauri events (Rust → React).
// Event names and payload shapes defined in docs/ARCHITECTURE.md §3.
// Implemented in Phase 1+ as streaming paths come online.

export const EVENT_WATERFALL_FRAME = "waterfall-frame";
export const EVENT_AUDIO_CHUNK = "audio-chunk";
export const EVENT_SIGNAL_LEVEL = "signal-level";
export const EVENT_DEVICE_STATUS = "device-status";
