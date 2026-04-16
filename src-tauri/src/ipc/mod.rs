//! Tauri IPC surface: commands (React → Rust) and binary events (Rust → React).
//!
//! Contract defined in `docs/ARCHITECTURE.md` §3.

pub mod commands;
pub mod events;
