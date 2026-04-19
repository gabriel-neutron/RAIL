//! Tauri IPC surface: commands (React → Rust) and binary events (Rust → React).
//!
//! Contract defined in `docs/ARCHITECTURE.md` §3.
//!
//! `commands` hosts the session lifecycle and tuning surface. The higher-rate
//! paths (capture, replay, DSP worker) are split into sibling modules so
//! `commands.rs` stays readable.

pub mod commands;
pub mod events;

pub(crate) mod capture_cmd;
pub(crate) mod dsp_task;
pub(crate) mod replay_cmd;
