//! Wideband scanner — sequential sweep over a configurable frequency range.
//!
//! The scanner reuses the live IQ stream: `retune` is called per step while
//! the DSP worker keeps running. Peak baseband power is polled from the shared
//! `latest_dbfs_bits` atomic written by the DSP task every buffer cycle.
//!
//! See `docs/TIMELINE.md` Phase 9 and `docs/ARCHITECTURE.md` §3.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytemuck::cast_slice;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime};

use crate::hardware::TunerHandle;
use crate::ipc::events::{ScanComplete, ScanStep, ScanStopped};

/// Arguments for [`start_scan`](crate::ipc::commands::start_scan).
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartScanArgs {
    /// First frequency to visit (Hz).
    pub start_hz: u32,
    /// Last frequency to visit, inclusive (Hz).
    pub stop_hz: u32,
    /// Frequency step between consecutive tuning points (Hz, ≥ 1 000).
    pub step_hz: u32,
    /// How long to dwell at each step before measuring (ms, ≥ 50).
    pub dwell_ms: u64,
    /// Optional squelch gate (dBFS). When a step's peak exceeds this,
    /// the scan stops and emits `scan-stopped`. `None` disables early-stop.
    #[serde(default)]
    pub squelch_dbfs: Option<f32>,
}

/// Reply for [`start_scan`](crate::ipc::commands::start_scan).
/// Lists every frequency the scanner will visit in order so the frontend
/// can pre-allocate its result buffer and map step index → Hz.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStartReply {
    pub frequencies_hz: Vec<u32>,
}

/// Running scanner task stored in [`crate::ipc::commands::AppState`].
pub(crate) struct ScannerHandle {
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) handle: tokio::task::JoinHandle<()>,
}

/// Build the ordered list of frequencies for a sweep, clamped to avoid
/// infinite loops on pathological inputs.
pub(crate) fn build_frequency_list(start_hz: u32, stop_hz: u32, step_hz: u32) -> Vec<u32> {
    let mut freqs = Vec::new();
    let mut f = start_hz;
    loop {
        freqs.push(f);
        let next = f.saturating_add(step_hz);
        if next > stop_hz || next == f {
            break;
        }
        f = next;
    }
    freqs
}

/// Spawn the scanner task and return its [`ScannerHandle`].
///
/// # Arguments
/// * `tuner` — copy of the live `TunerHandle`; the scan task calls
///   `set_center_freq` without interrupting the IQ reader thread.
/// * `lo_offset_hz` — `sample_rate / 4` LO offset (see `docs/DSP.md` §1).
/// * `max_dbfs_per_bin` — per-bin peak accumulator maintained by the DSP task.
///   The scanner resets it after settle and reads the peak-of-bins at dwell end,
///   capturing burst signals that would be missed by a single-poll approach.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_scanner<R: Runtime>(
    app: AppHandle<R>,
    tuner: TunerHandle,
    lo_offset_hz: u32,
    frequencies_hz: Vec<u32>,
    dwell_ms: u64,
    squelch_dbfs: Option<f32>,
    max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
    scan_channel: Channel<InvokeResponseBody>,
) -> ScannerHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_task = cancel.clone();
    let handle = tokio::spawn(async move {
        run_scanner(
            app,
            tuner,
            lo_offset_hz,
            frequencies_hz,
            dwell_ms,
            squelch_dbfs,
            max_dbfs_per_bin,
            scan_channel,
            cancel_task,
        )
        .await;
    });
    ScannerHandle { cancel, handle }
}

/// Poll interval during the measurement window after hardware has settled.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How long to wait after a retune before starting peak tracking.
/// RTL-SDR flushes its USB buffer in ~16 ms; 40 ms gives 2× margin for
/// scheduling jitter so stale samples from the previous step cannot
/// pollute the measurement (docs/HARDWARE.md §2 — settle time).
const SETTLE_MS: Duration = Duration::from_millis(40);

#[allow(clippy::too_many_arguments)]
async fn run_scanner<R: Runtime>(
    app: AppHandle<R>,
    tuner: TunerHandle,
    lo_offset_hz: u32,
    frequencies_hz: Vec<u32>,
    dwell_ms: u64,
    squelch_dbfs: Option<f32>,
    max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
    scan_channel: Channel<InvokeResponseBody>,
    cancel: Arc<AtomicBool>,
) {
    let dwell = Duration::from_millis(dwell_ms);
    let mut stopped_at: Option<u32> = None;

    for &freq_hz in &frequencies_hz {
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        // Retune: park LO at freq − fs/4 (docs/DSP.md §1)
        if let Err(e) = tuner.set_center_freq(freq_hz.saturating_sub(lo_offset_hz)) {
            log::warn!("scanner: retune to {freq_hz} Hz failed: {e}");
            emit_step(&scan_channel, f32::NEG_INFINITY);
            continue;
        }
        // Notify the frontend so all display components (FrequencyAxis,
        // FilterBandMarker, FrequencyControl) stay in sync via the radio store.
        if let Err(e) = (ScanStep { frequency_hz: freq_hz }).emit(&app) {
            log::warn!("scanner: scan-step emit failed: {e}");
        }

        // Settle: wait for the RTL-SDR to flush old-frequency samples.
        // Do not measure during this window (docs/HARDWARE.md §2).
        tokio::time::sleep(SETTLE_MS).await;
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        // Reset accumulator at the start of the measurement window so that
        // power from the previous step or the settle transient is discarded.
        if let Ok(mut acc) = max_dbfs_per_bin.lock() {
            acc.iter_mut().for_each(|v| *v = f32::NEG_INFINITY);
        }

        // Dwell: the DSP task continuously updates max_dbfs_per_bin; we only
        // need to sleep and check for cancellation. Peak-of-bins is read once
        // at the end of the window, capturing bursts that span any fraction of
        // the dwell interval — including sub-POLL_INTERVAL events.
        let mut elapsed = Duration::ZERO;
        while elapsed < dwell {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
            elapsed += POLL_INTERVAL;
        }

        let peak_dbfs = {
            let acc = max_dbfs_per_bin.lock().unwrap_or_else(|e| e.into_inner());
            acc.iter().copied().filter(|v| v.is_finite()).fold(f32::NEG_INFINITY, f32::max)
        };

        emit_step(&scan_channel, peak_dbfs);

        if let Some(threshold) = squelch_dbfs {
            if peak_dbfs > threshold {
                stopped_at = Some(freq_hz);
                break;
            }
        }
    }

    if let Some(freq_hz) = stopped_at {
        if let Err(e) = (ScanStopped { frequency_hz: freq_hz }).emit(&app) {
            log::warn!("scanner: scan-stopped emit failed: {e}");
        }
    } else {
        if let Err(e) = ScanComplete.emit(&app) {
            log::warn!("scanner: scan-complete emit failed: {e}");
        }
    }
}

/// Emit one `f32` (4 bytes, little-endian) on the scan binary channel.
/// Each call corresponds to one frequency step's peak dBFS.
fn emit_step(channel: &Channel<InvokeResponseBody>, peak_dbfs: f32) {
    let bytes: &[u8] = cast_slice(std::slice::from_ref(&peak_dbfs));
    if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
        log::warn!("scanner: channel send failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::build_frequency_list;

    #[test]
    fn frequency_list_inclusive_stop() {
        let freqs = build_frequency_list(87_500_000, 108_000_000, 200_000);
        assert_eq!(freqs[0], 87_500_000);
        assert_eq!(*freqs.last().unwrap(), 107_900_000);
        assert_eq!(freqs.len(), 103);
    }

    #[test]
    fn frequency_list_single_step() {
        let freqs = build_frequency_list(100_000_000, 100_000_000, 200_000);
        assert_eq!(freqs, vec![100_000_000]);
    }

    #[test]
    fn frequency_list_exact_stop() {
        let freqs = build_frequency_list(100_000_000, 100_400_000, 200_000);
        assert_eq!(freqs, vec![100_000_000, 100_200_000, 100_400_000]);
    }
}
