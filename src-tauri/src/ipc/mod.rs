//! Tauri IPC surface: commands (React → Rust) and binary events (Rust → React).
//!
//! Contract defined in `docs/ARCHITECTURE.md` §3.
//!
//! `commands` hosts the tuning surface and teardown. Session assembly
//! lives in `session`; the higher-rate paths (capture, replay, DSP
//! worker) are split into sibling modules so `commands.rs` stays
//! readable.

pub mod commands;
pub mod event_contract;
pub mod events;
pub mod session;

pub(crate) mod capture_cmd;
pub(crate) mod control;
pub(crate) mod dsp_task;
pub(crate) mod replay_cmd;
