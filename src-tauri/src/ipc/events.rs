//! Rust → React event payloads and constructors.
//!
//! Streaming (waterfall frames) uses a `tauri::ipc::Channel<InvokeResponseBody>`
//! opened by the `start_stream` command — that path never touches JSON.
//!
//! Low-rate status updates (device connect/disconnect) use the regular JSON
//! event bus. See `docs/ARCHITECTURE.md` §3.
//!
//! The payload structs, their wire-name constants and their [`IpcEvent`] impls
//! are generated from `shared/ipc_events.json` by
//! `scripts/gen-ipc-events.mjs`; the emit adapter lives once in
//! [`crate::ipc::event_contract`]. Only the constructors below are hand-written.

use serde::Serialize;

use crate::ipc::event_contract::IpcEvent;

include!("generated/events.rs");

impl DeviceStatus {
    /// A device that is open and streaming.
    pub fn connected() -> Self {
        Self {
            connected: true,
            error: None,
        }
    }

    /// A device that dropped out, carrying the reason for the frontend.
    pub fn disconnected_with(err: impl Into<String>) -> Self {
        Self {
            connected: false,
            error: Some(err.into()),
        }
    }
}

impl SignalLevel {
    /// `current` and `peak` are in dBFS.
    pub fn new(current: f32, peak: f32) -> Self {
        Self { current, peak }
    }
}

impl ReplayPosition {
    /// `sample_idx` is in IQ samples; `position_ms` and `total_ms` in milliseconds.
    pub fn new(sample_idx: u64, position_ms: u64, total_ms: u64, playing: bool) -> Self {
        Self {
            sample_idx,
            position_ms,
            total_ms,
            playing,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn keys(value: &serde_json::Value) -> Vec<String> {
        let mut k: Vec<String> = value
            .as_object()
            .expect("payload must serialise to a JSON object")
            .keys()
            .cloned()
            .collect();
        k.sort();
        k
    }

    // The wire is what the frontend types in `src/ipc/generated/events.ts`
    // claim it is. Nothing else in the test suite exercises serialisation, so
    // a generator change that reshapes a payload would otherwise be silent.

    #[test]
    fn device_status_omits_absent_error() {
        let v = serde_json::to_value(DeviceStatus::connected()).expect("serialise");
        assert_eq!(keys(&v), vec!["connected"]);
        assert_eq!(v["connected"], serde_json::json!(true));
    }

    #[test]
    fn device_status_carries_present_error() {
        let v = serde_json::to_value(DeviceStatus::disconnected_with("gone")).expect("serialise");
        assert_eq!(keys(&v), vec!["connected", "error"]);
        assert_eq!(v["error"], serde_json::json!("gone"));
    }

    #[test]
    fn signal_level_keys() {
        let v = serde_json::to_value(SignalLevel::new(-30.0, -12.0)).expect("serialise");
        assert_eq!(keys(&v), vec!["current", "peak"]);
    }

    #[test]
    fn scan_step_and_stopped_use_camel_case() {
        let step = serde_json::to_value(ScanStep {
            frequency_hz: 100_000_000,
        })
        .expect("serialise");
        assert_eq!(keys(&step), vec!["frequencyHz"]);

        let stopped = serde_json::to_value(ScanStopped {
            frequency_hz: 100_000_000,
        })
        .expect("serialise");
        assert_eq!(keys(&stopped), vec!["frequencyHz"]);
    }

    #[test]
    fn scan_complete_serialises_to_an_empty_object() {
        let v = serde_json::to_value(ScanComplete {}).expect("serialise");
        assert_eq!(v, serde_json::json!({}));
    }

    #[test]
    fn replay_position_uses_camel_case() {
        let v = serde_json::to_value(ReplayPosition::new(1, 2, 3, true)).expect("serialise");
        assert_eq!(
            keys(&v),
            vec!["playing", "positionMs", "sampleIdx", "totalMs"]
        );
    }

    #[test]
    fn signal_classification_keeps_null_confirmed() {
        let v = serde_json::to_value(SignalClassification {
            confirmed: None,
            candidates: vec!["FM"],
            reason: "low snr".to_string(),
        })
        .expect("serialise");
        assert_eq!(keys(&v), vec!["candidates", "confirmed", "reason"]);
        assert_eq!(v["confirmed"], serde_json::Value::Null);
    }

    #[test]
    fn wire_names_match_the_constants() {
        assert_eq!(DeviceStatus::NAME, EVENT_DEVICE_STATUS);
        assert_eq!(SignalLevel::NAME, EVENT_SIGNAL_LEVEL);
        assert_eq!(ScanStep::NAME, EVENT_SCAN_STEP);
        assert_eq!(ScanComplete::NAME, EVENT_SCAN_COMPLETE);
        assert_eq!(ScanStopped::NAME, EVENT_SCAN_STOPPED);
        assert_eq!(SignalClassification::NAME, EVENT_SIGNAL_CLASSIFICATION);
        assert_eq!(ReplayPosition::NAME, EVENT_REPLAY_POSITION);
    }
}
