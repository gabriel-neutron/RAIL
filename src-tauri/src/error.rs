//! Central error type for the RAIL backend.
//!
//! Defined in `docs/ARCHITECTURE.md` §6. All public backend functions
//! return `Result<T, RailError>` and errors surface to the frontend via
//! Tauri command responses or the `device-status` event.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum RailError {
    #[error("RTL-SDR device not found")]
    DeviceNotFound,

    #[error("failed to open RTL-SDR device: {0}")]
    DeviceOpenFailed(String),

    #[error("stream error: {0}")]
    StreamError(String),

    #[error("DSP error: {0}")]
    DspError(String),

    #[error("capture error: {0}")]
    CaptureError(String),

    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
}
