//! Wideband scanner — sequential sweep over a configurable frequency range.
//!
//! The scanner reuses the live IQ stream: `retune` is called per step while
//! the DSP worker keeps running. Per-step SNR is computed from
//! `max_dbfs_per_bin`: average power in a narrowband window centred on the
//! target frequency minus the median of the full spectrum (noise floor
//! estimate). See `docs/DSP.md` §2 for bin geometry.
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
    /// Optional early-stop SNR gate (dB). When a step's local SNR exceeds
    /// this, the scan stops and emits `scan-stopped`. `None` disables
    /// early-stop and always completes the full sweep.
    #[serde(default)]
    pub squelch_snr_db: Option<f32>,
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

/// Compute the local SNR for one scan step from the per-bin peak accumulator.
///
/// Returns `(signal_avg_db, noise_floor_db)` where:
/// - `signal_avg_db` is the mean of finite accumulator values in the
///   narrowband window `[center − half_bins, center + half_bins]` (the
///   channel centred on the tuned frequency after the `fs/4` shift).
/// - `noise_floor_db` is the median of all finite accumulator values
///   (robust to sparse signal peaks — see `docs/DSP.md` §2 for bin geometry).
///
/// Both return `f32::NEG_INFINITY` when the accumulator is empty or has
/// fewer than 16 finite values.
fn compute_channel_snr(acc: &[f32], sample_rate_hz: u32, step_hz: u32) -> (f32, f32) {
    let fft_size = acc.len();
    if fft_size == 0 {
        return (f32::NEG_INFINITY, f32::NEG_INFINITY);
    }

    // After fs/4 downconversion the target frequency lands at the centre bin.
    let center = fft_size / 2;
    let bin_width = sample_rate_hz as f32 / fft_size as f32;
    let half_bins = ((step_hz as f32 / 2.0) / bin_width) as usize;
    let half_bins = half_bins.max(1).min(center.saturating_sub(1));
    let lo = center.saturating_sub(half_bins);
    let hi = (center + half_bins).min(fft_size - 1);

    // Signal: average of target-window bins.
    let mut sig_sum = 0.0_f32;
    let mut sig_n = 0usize;
    for &v in &acc[lo..=hi] {
        if v.is_finite() {
            sig_sum += v;
            sig_n += 1;
        }
    }
    let signal_avg_db = if sig_n == 0 {
        f32::NEG_INFINITY
    } else {
        sig_sum / sig_n as f32
    };

    // Noise: median of all finite bins (robust to sparse peaks).
    let mut all_finite: Vec<f32> = acc.iter().copied().filter(|v| v.is_finite()).collect();
    if all_finite.len() < 16 {
        return (signal_avg_db, f32::NEG_INFINITY);
    }
    all_finite.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let noise_floor_db = all_finite[all_finite.len() / 2];

    (signal_avg_db, noise_floor_db)
}

/// Spawn the scanner task and return its [`ScannerHandle`].
///
/// # Arguments
/// * `tuner` — copy of the live `TunerHandle`; the scan task calls
///   `set_center_freq` without interrupting the IQ reader thread.
/// * `lo_offset_hz` — `sample_rate / 4` LO offset (see `docs/DSP.md` §1).
/// * `max_dbfs_per_bin` — per-bin peak accumulator maintained by the DSP task.
///   The scanner resets it after settle and reads the per-channel average at
///   dwell end.
/// * `sample_rate_hz` — SDR sample rate, used to derive FFT bin width.
/// * `step_hz` — frequency step, used to derive the channel measurement window.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_scanner<R: Runtime>(
    app: AppHandle<R>,
    tuner: TunerHandle,
    lo_offset_hz: u32,
    frequencies_hz: Vec<u32>,
    dwell_ms: u64,
    squelch_snr_db: Option<f32>,
    max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
    scan_channel: Channel<InvokeResponseBody>,
    sample_rate_hz: u32,
    step_hz: u32,
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
            squelch_snr_db,
            max_dbfs_per_bin,
            scan_channel,
            cancel_task,
            sample_rate_hz,
            step_hz,
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
    squelch_snr_db: Option<f32>,
    max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
    scan_channel: Channel<InvokeResponseBody>,
    cancel: Arc<AtomicBool>,
    sample_rate_hz: u32,
    step_hz: u32,
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
            emit_step(&scan_channel, f32::NEG_INFINITY, f32::NEG_INFINITY);
            continue;
        }
        // Notify the frontend so all display components (FrequencyAxis,
        // FilterBandMarker, FrequencyControl) stay in sync via the radio store.
        if let Err(e) = (ScanStep {
            frequency_hz: freq_hz,
        })
        .emit(&app)
        {
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
        // need to sleep and check for cancellation.
        let mut elapsed = Duration::ZERO;
        while elapsed < dwell {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
            elapsed += POLL_INTERVAL;
        }

        let (signal_avg_db, noise_floor_db) = {
            let acc = max_dbfs_per_bin.lock().unwrap_or_else(|e| e.into_inner());
            compute_channel_snr(&acc, sample_rate_hz, step_hz)
        };

        emit_step(&scan_channel, signal_avg_db, noise_floor_db);

        if let Some(threshold) = squelch_snr_db {
            let snr = signal_avg_db - noise_floor_db;
            if snr.is_finite() && snr > threshold {
                stopped_at = Some(freq_hz);
                break;
            }
        }
    }

    if let Some(freq_hz) = stopped_at {
        if let Err(e) = (ScanStopped {
            frequency_hz: freq_hz,
        })
        .emit(&app)
        {
            log::warn!("scanner: scan-stopped emit failed: {e}");
        }
    } else if let Err(e) = ScanComplete.emit(&app) {
        log::warn!("scanner: scan-complete emit failed: {e}");
    }
}

/// Emit one step result (8 bytes, two little-endian f32) on the scan channel.
/// Byte 0–3: `signal_avg_db` (average power in target channel window).
/// Byte 4–7: `noise_floor_db` (median of full spectrum — noise reference).
/// Frontend computes SNR as the difference of the two fields.
fn emit_step(channel: &Channel<InvokeResponseBody>, signal_avg_db: f32, noise_floor_db: f32) {
    let payload = [signal_avg_db, noise_floor_db];
    let bytes: &[u8] = cast_slice(&payload);
    if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
        log::warn!("scanner: channel send failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::{build_frequency_list, compute_channel_snr};

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

    #[test]
    fn channel_snr_detects_elevated_window() {
        // 8192 bins all at -50 dBFS; signal window at -20 dBFS.
        // Expected: signal_avg ≈ -20, noise_floor ≈ -50, SNR ≈ 30.
        let fs = 2_048_000_u32;
        let step = 200_000_u32;
        let n = 8192_usize;
        let mut acc = vec![-50.0_f32; n];

        // Paint the target window: bins [3696, 4496] at -20 dBFS.
        let center = n / 2; // 4096
        let bin_width = fs as f32 / n as f32; // 250 Hz
        let half = ((step as f32 / 2.0) / bin_width) as usize; // 400
        for v in &mut acc[(center - half)..=(center + half)] {
            *v = -20.0;
        }

        let (sig, noise) = compute_channel_snr(&acc, fs, step);
        assert!(
            (sig - (-20.0)).abs() < 0.5,
            "signal_avg_db should be ~-20, got {sig}"
        );
        assert!(
            (noise - (-50.0)).abs() < 1.0,
            "noise_floor_db should be ~-50, got {noise}"
        );
        assert!(
            (sig - noise - 30.0).abs() < 1.5,
            "SNR should be ~30 dB, got {}",
            sig - noise
        );
    }

    #[test]
    fn channel_snr_all_neg_infinity() {
        let acc = vec![f32::NEG_INFINITY; 8192];
        let (sig, noise) = compute_channel_snr(&acc, 2_048_000, 200_000);
        assert!(!sig.is_finite(), "signal should be NEG_INFINITY");
        assert!(!noise.is_finite(), "noise should be NEG_INFINITY");
    }

    #[test]
    fn channel_snr_empty_accumulator() {
        let (sig, noise) = compute_channel_snr(&[], 2_048_000, 200_000);
        assert!(!sig.is_finite());
        assert!(!noise.is_finite());
    }
}
