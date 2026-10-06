//! Central error type for the RAIL backend.
//!
//! Defined in `docs/ARCHITECTURE.md` §6. All public backend functions
//! return `Result<T, RailError>` and errors surface to the frontend via
//! Tauri command responses or the `device-status` event.

use serde::Serialize;
use thiserror::Error;

/// Every failure the RAIL backend reports to the frontend.
///
/// The `#[error]` string is the user-facing text; each variant below says when it is raised.
#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum RailError {
    /// No RTL-SDR device is attached, or none matched the requested index.
    #[error("RTL-SDR device not found")]
    DeviceNotFound,

    /// A device was found but librtlsdr refused to open or configure it.
    #[error("failed to open RTL-SDR device: {0}")]
    DeviceOpenFailed(String),

    /// The IQ reader thread or a session channel failed, including lock poisoning.
    #[error("stream error: {0}")]
    StreamError(String),

    /// A DSP stage could not run, e.g. an unsupported rate or a failed FFT setup.
    #[error("DSP error: {0}")]
    DspError(String),

    /// A capture file could not be written, serialized or finalized.
    #[error("capture error: {0}")]
    CaptureError(String),

    /// A command argument was missing, out of range, or invalid for the current session.
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
}
