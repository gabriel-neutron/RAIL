//! Rust → React event payloads and emit helpers.
//!
//! Streaming (waterfall frames) uses a `tauri::ipc::Channel<InvokeResponseBody>`
//! opened by the `start_stream` command — that path never touches JSON.
//!
//! Low-rate status updates (device connect/disconnect) use the regular
//! JSON event bus via [`DeviceStatus::emit`]. See `docs/ARCHITECTURE.md` §3.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::error::RailError;

/// Event name for device connection updates (`docs/ARCHITECTURE.md` §3).
pub const EVENT_DEVICE_STATUS: &str = "device-status";

/// Payload for the `device-status` JSON event.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DeviceStatus {
    pub fn connected() -> Self {
        Self {
            connected: true,
            error: None,
        }
    }

    pub fn disconnected_with(err: impl Into<String>) -> Self {
        Self {
            connected: false,
            error: Some(err.into()),
        }
    }

    pub fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError> {
        app.emit(EVENT_DEVICE_STATUS, self)
            .map_err(|e| RailError::StreamError(format!("emit device-status: {e}")))
    }
}
