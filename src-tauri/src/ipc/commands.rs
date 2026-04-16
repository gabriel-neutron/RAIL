//! Tauri command handlers.
//!
//! Request/response surface from React. Streaming data uses binary events
//! defined in `ipc::events` (see `docs/ARCHITECTURE.md` §3).

use crate::error::RailError;
use crate::hardware::{self, DeviceInfo};

/// Liveness check: returns `"pong"`. Used by the frontend on startup to
/// verify the IPC bridge is healthy.
#[tauri::command]
pub fn ping() -> &'static str {
    "pong"
}

/// Enumerate attached RTL-SDR compatible USB devices.
/// Returns the first match or `RailError::DeviceNotFound`.
#[tauri::command]
pub fn check_device() -> Result<DeviceInfo, RailError> {
    hardware::check_device()
}
