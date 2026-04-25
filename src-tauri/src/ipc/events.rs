//! Rust → React event payloads and emit helpers.
//!
//! Streaming (waterfall frames) uses a `tauri::ipc::Channel<InvokeResponseBody>`
//! opened by the `start_stream` command — that path never touches JSON.
//!
//! Low-rate status updates (device connect/disconnect) use the regular
//! JSON event bus via [`DeviceStatus::emit`]. See `docs/ARCHITECTURE.md` §3.

include!(concat!(env!("OUT_DIR"), "/generated_ipc_event_names.rs"));

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::error::RailError;

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

/// Payload for the `signal-level` JSON event. `current` and `peak`
/// are in dBFS (post-channel-filter baseband RMS; see
/// `docs/DSP.md` §2 and `DemodChain::process`).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SignalLevel {
    pub current: f32,
    pub peak: f32,
}

impl SignalLevel {
    pub fn new(current: f32, peak: f32) -> Self {
        Self { current, peak }
    }

    pub fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError> {
        app.emit(EVENT_SIGNAL_LEVEL, self)
            .map_err(|e| RailError::StreamError(format!("emit signal-level: {e}")))
    }
}

/// Payload for the `scan-step` JSON event. Emitted after each successful
/// retune so the frontend can keep `radioStore.frequencyHz` in sync with
/// the hardware. All display components (FrequencyAxis, FilterBandMarker,
/// FrequencyControl) read from that store and update automatically.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStep {
    /// The logical target frequency (Hz) — after lo-offset correction,
    /// matching what the user sees as the tuned centre.
    pub frequency_hz: u32,
}

impl ScanStep {
    pub fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError> {
        app.emit(EVENT_SCAN_STEP, self)
            .map_err(|e| RailError::StreamError(format!("emit scan-step: {e}")))
    }
}

/// Payload for the `scan-complete` JSON event. Emitted when a full sweep
/// finishes without hitting the squelch threshold (see `docs/TIMELINE.md` Phase 9).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ScanComplete;

impl ScanComplete {
    /// Emit `scan-complete` to all frontend windows.
    pub fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError> {
        app.emit(EVENT_SCAN_COMPLETE, self)
            .map_err(|e| RailError::StreamError(format!("emit scan-complete: {e}")))
    }
}

/// Payload for the `scan-stopped` JSON event. Emitted when the scanner
/// halts early because a step's peak power exceeded the squelch threshold.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStopped {
    /// The frequency (Hz) at which the signal was detected.
    pub frequency_hz: u32,
}

impl ScanStopped {
    /// Emit `scan-stopped` to all frontend windows.
    pub fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError> {
        app.emit(EVENT_SCAN_STOPPED, self)
            .map_err(|e| RailError::StreamError(format!("emit scan-stopped: {e}")))
    }
}

/// Payload for the `signal-classification` JSON event.
///
/// Emitted at ~2 Hz by the DSP task. See `docs/SIGNALS.md §5.4`.
///
/// - `confirmed`: wire-name of the spectrally confirmed mode (`"FM"` /
///   `"NFM"` / `"AM"` / `"USB"` / `"LSB"` / `"CW"`), or `null` when SNR is
///   too low. Maps to a green ModeSelector button.
/// - `candidates`: wire-names from the frequency prior; always populated for
///   known bands regardless of signal strength. Map to yellow buttons.
#[derive(Debug, Clone, Serialize)]
pub struct SignalClassification {
    pub confirmed: Option<&'static str>,
    pub candidates: Vec<&'static str>,
    pub reason: String,
}

impl SignalClassification {
    pub fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError> {
        app.emit(EVENT_SIGNAL_CLASSIFICATION, self)
            .map_err(|e| RailError::StreamError(format!("emit signal-classification: {e}")))
    }
}

/// Payload for the `replay-position` JSON event. Emitted at ~25 Hz
/// by the replay reader so the transport slider stays in sync with
/// the IQ file read head (see [`crate::replay`]).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayPosition {
    pub sample_idx: u64,
    pub position_ms: u64,
    pub total_ms: u64,
    pub playing: bool,
}

impl ReplayPosition {
    pub fn new(sample_idx: u64, position_ms: u64, total_ms: u64, playing: bool) -> Self {
        Self {
            sample_idx,
            position_ms,
            total_ms,
            playing,
        }
    }

    pub fn emit<R: Runtime>(&self, app: &AppHandle<R>) -> Result<(), RailError> {
        app.emit(EVENT_REPLAY_POSITION, self)
            .map_err(|e| RailError::StreamError(format!("emit replay-position: {e}")))
    }
}
