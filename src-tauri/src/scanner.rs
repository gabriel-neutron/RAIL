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

use crate::hardware::Tuner;
use crate::ipc::control::{DspControl, DspControlHandle};
use crate::ipc::event_contract::Emit;
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

/// Running scanner task stored in [`crate::ipc::session::AppState`].
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
    all_finite.sort_unstable_by(|a, b| a.total_cmp(b));
    let noise_floor_db = all_finite[all_finite.len() / 2];

    (signal_avg_db, noise_floor_db)
}

/// One thing worth telling the frontend about during a sweep. The sink that
/// [`run_scanner`] writes these to is supplied by [`spawn_scanner`], which is
/// the only place Tauri's `AppHandle`/`Channel` appear — that is what lets the
/// sweep be tested with no dongle and no Tauri runtime.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ScanEvent {
    /// The sweep has tuned to `frequency_hz` (the user-facing frequency, not
    /// the LO).
    Tuned { frequency_hz: u32 },
    /// One step result. Exactly one is emitted per visited frequency, in
    /// order — the frontend maps results to frequencies positionally.
    Measured {
        signal_avg_db: f32,
        noise_floor_db: f32,
    },
    /// Squelch fired: the sweep stopped early at `frequency_hz`.
    Stopped { frequency_hz: u32 },
    /// The sweep visited every frequency without stopping.
    Complete,
}

/// Spawn the scanner task and return its [`ScannerHandle`].
///
/// # Arguments
/// * `tuner` — [`Tuner`] port for the live device; the scan task calls
///   `set_center_freq` without interrupting the IQ reader thread.
/// * `lo_offset_hz` — `sample_rate / 4` LO offset (see `docs/DSP.md` §1).
/// * `max_dbfs_per_bin` — per-bin peak accumulator maintained by the DSP task.
///   The scanner resets it after settle and reads the per-channel average at
///   dwell end.
/// * `sample_rate_hz` — SDR sample rate, used to derive FFT bin width.
/// * `step_hz` — frequency step, used to derive the channel measurement window.
/// * `control` — the DSP control seam. Every step sends
///   [`DspControl::Retune`] so the worker flushes its spectral accumulator
///   and relabels the classifier and capture metadata.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_scanner<R: Runtime, T: Tuner + Send + 'static>(
    app: AppHandle<R>,
    tuner: T,
    lo_offset_hz: u32,
    frequencies_hz: Vec<u32>,
    dwell_ms: u64,
    squelch_snr_db: Option<f32>,
    max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
    scan_channel: Channel<InvokeResponseBody>,
    sample_rate_hz: u32,
    step_hz: u32,
    control: DspControlHandle,
) -> ScannerHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_task = cancel.clone();
    let handle = tokio::spawn(async move {
        let emit = move |event: ScanEvent| match event {
            // Tell the DSP worker the centre moved (it flushes its spectral
            // accumulator and relabels the classifier and capture metadata),
            // then notify the frontend so all display components
            // (FrequencyAxis, FilterBandMarker, FrequencyControl) stay in
            // sync via the radio store.
            ScanEvent::Tuned { frequency_hz } => {
                if let Some(msg) = control_for_scan_event(&event) {
                    if let Err(e) = control.send(msg) {
                        log::warn!("scanner: control send failed: {e}");
                    }
                }
                if let Err(e) = (ScanStep { frequency_hz }).emit(&app) {
                    log::warn!("scanner: scan-step emit failed: {e}");
                }
            }
            ScanEvent::Measured {
                signal_avg_db,
                noise_floor_db,
            } => emit_step(&scan_channel, signal_avg_db, noise_floor_db),
            ScanEvent::Stopped { frequency_hz } => {
                if let Err(e) = (ScanStopped { frequency_hz }).emit(&app) {
                    log::warn!("scanner: scan-stopped emit failed: {e}");
                }
            }
            ScanEvent::Complete => {
                if let Err(e) = (ScanComplete {}).emit(&app) {
                    log::warn!("scanner: scan-complete emit failed: {e}");
                }
            }
        };
        run_scanner(
            tuner,
            lo_offset_hz,
            frequencies_hz,
            dwell_ms,
            squelch_snr_db,
            max_dbfs_per_bin,
            cancel_task,
            sample_rate_hz,
            step_hz,
            emit,
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

/// Map a sweep event onto the DSP control message it implies, if any.
/// Only a retune concerns the worker; measurement and completion events
/// are frontend-facing. Pure so the mapping is testable on its own.
fn control_for_scan_event(event: &ScanEvent) -> Option<DspControl> {
    match *event {
        ScanEvent::Tuned { frequency_hz } => Some(DspControl::Retune {
            center_hz: frequency_hz,
        }),
        _ => None,
    }
}

/// Run one sweep, reporting progress through `emit`.
///
/// Free of Tauri types on purpose: the sink is a plain closure, so the sweep
/// sequencing is testable against a fake [`Tuner`] with no dongle attached.
///
/// Timing note: `ScanEvent::Tuned` is emitted the instant the tuner accepts
/// the new frequency, but the worker drains the resulting `Retune` behind up
/// to `IQ_CHANNEL_CAPACITY` queued IQ chunks plus the USB buffers in flight —
/// roughly 60–100 ms of already-captured old-centre samples. Message order is
/// not sample order. Measurement is unaffected (the accumulator is reset here
/// after `SETTLE_MS`); what the message buys is correct-per-step classifier
/// and capture metadata instead of stale-for-the-whole-sweep.
#[allow(clippy::too_many_arguments)]
async fn run_scanner<T: Tuner, F: FnMut(ScanEvent) + Send>(
    tuner: T,
    lo_offset_hz: u32,
    frequencies_hz: Vec<u32>,
    dwell_ms: u64,
    squelch_snr_db: Option<f32>,
    max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
    cancel: Arc<AtomicBool>,
    sample_rate_hz: u32,
    step_hz: u32,
    mut emit: F,
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
            emit(ScanEvent::Measured {
                signal_avg_db: f32::NEG_INFINITY,
                noise_floor_db: f32::NEG_INFINITY,
            });
            continue;
        }
        emit(ScanEvent::Tuned {
            frequency_hz: freq_hz,
        });

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

        emit(ScanEvent::Measured {
            signal_avg_db,
            noise_floor_db,
        });

        if let Some(threshold) = squelch_snr_db {
            let snr = signal_avg_db - noise_floor_db;
            if snr.is_finite() && snr > threshold {
                stopped_at = Some(freq_hz);
                break;
            }
        }
    }

    match stopped_at {
        Some(frequency_hz) => emit(ScanEvent::Stopped { frequency_hz }),
        None => emit(ScanEvent::Complete),
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{
        build_frequency_list, compute_channel_snr, control_for_scan_event, run_scanner, DspControl,
        ScanEvent,
    };
    use crate::hardware::fake_tuner::FakeTuner;

    const FS_HZ: u32 = 2_048_000;
    const LO_OFFSET_HZ: u32 = FS_HZ / 4;
    const STEP_HZ: u32 = 200_000;
    /// Tuner resolution the fake snaps read-back to (docs/HARDWARE.md §4).
    const SNAP_HZ: u32 = 1_000;

    /// Dwell long enough for the poll loop to run at least once after the
    /// scanner clears the accumulator; the stand-in DSP task refills it.
    const DWELL_MS: u64 = 60;
    const NOISE_DB: f32 = -50.0;
    const SIGNAL_DB: f32 = -10.0;

    fn accumulator() -> Arc<Mutex<Vec<f32>>> {
        Arc::new(Mutex::new(vec![f32::NEG_INFINITY; 8192]))
    }

    /// Stand in for the DSP task, which keeps writing per-bin peaks while the
    /// scanner dwells. `signal` paints the target channel above the floor so
    /// squelch fires; otherwise the spectrum is flat noise.
    fn spawn_dsp_filler(acc: Arc<Mutex<Vec<f32>>>, signal: bool) {
        tokio::spawn(async move {
            loop {
                {
                    let mut guard = acc.lock().unwrap();
                    let n = guard.len();
                    guard.iter_mut().for_each(|v| *v = NOISE_DB);
                    if signal {
                        let center = n / 2;
                        let half = ((STEP_HZ as f32 / 2.0) / (FS_HZ as f32 / n as f32)) as usize;
                        for v in &mut guard[(center - half)..=(center + half)] {
                            *v = SIGNAL_DB;
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        });
    }

    #[test]
    fn tuned_event_maps_to_retune_control() {
        // Composed with `sweep_tunes_every_frequency_at_lo_offset` (which
        // already asserts one Tuned per frequency), this covers "the scanner
        // sends Retune on every step" without threading a channel through
        // the Tauri-free sweep.
        let msg = control_for_scan_event(&ScanEvent::Tuned {
            frequency_hz: 433_920_000,
        });
        assert!(matches!(
            msg,
            Some(DspControl::Retune {
                center_hz: 433_920_000
            })
        ));

        // Nothing else concerns the worker.
        assert!(control_for_scan_event(&ScanEvent::Measured {
            signal_avg_db: -10.0,
            noise_floor_db: -50.0,
        })
        .is_none());
        assert!(control_for_scan_event(&ScanEvent::Stopped {
            frequency_hz: 433_920_000,
        })
        .is_none());
        assert!(control_for_scan_event(&ScanEvent::Complete).is_none());
    }

    fn measured_count(events: &[ScanEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, ScanEvent::Measured { .. }))
            .count()
    }

    #[tokio::test]
    async fn sweep_tunes_every_frequency_at_lo_offset() {
        let freqs = vec![100_000_000_u32, 100_200_000, 100_400_000];
        let tuner = FakeTuner::new(SNAP_HZ);
        let acc = accumulator();
        spawn_dsp_filler(acc.clone(), false);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();

        run_scanner(
            &tuner,
            LO_OFFSET_HZ,
            freqs.clone(),
            DWELL_MS,
            None,
            acc,
            Arc::new(AtomicBool::new(false)),
            FS_HZ,
            STEP_HZ,
            move |e| sink.lock().unwrap().push(e),
        )
        .await;

        assert_eq!(
            tuner.tuned_hz(),
            freqs.iter().map(|f| f - LO_OFFSET_HZ).collect::<Vec<_>>(),
            "sweep must visit every frequency in order, parked fs/4 low"
        );

        let events = events.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![
                ScanEvent::Tuned {
                    frequency_hz: freqs[0]
                },
                ScanEvent::Measured {
                    signal_avg_db: NOISE_DB,
                    noise_floor_db: NOISE_DB
                },
                ScanEvent::Tuned {
                    frequency_hz: freqs[1]
                },
                ScanEvent::Measured {
                    signal_avg_db: NOISE_DB,
                    noise_floor_db: NOISE_DB
                },
                ScanEvent::Tuned {
                    frequency_hz: freqs[2]
                },
                ScanEvent::Measured {
                    signal_avg_db: NOISE_DB,
                    noise_floor_db: NOISE_DB
                },
                ScanEvent::Complete,
            ]
        );
    }

    #[tokio::test]
    async fn retune_failure_emits_placeholder_and_continues() {
        let freqs = vec![100_000_000_u32, 100_200_000, 100_400_000];
        let tuner = FakeTuner::failing_at(SNAP_HZ, freqs[1] - LO_OFFSET_HZ);
        let acc = accumulator();
        spawn_dsp_filler(acc.clone(), false);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();

        run_scanner(
            &tuner,
            LO_OFFSET_HZ,
            freqs.clone(),
            DWELL_MS,
            None,
            acc,
            Arc::new(AtomicBool::new(false)),
            FS_HZ,
            STEP_HZ,
            move |e| sink.lock().unwrap().push(e),
        )
        .await;

        let events = events.lock().unwrap().clone();
        assert!(
            !events.contains(&ScanEvent::Tuned {
                frequency_hz: freqs[1]
            }),
            "a failed retune must not report the frequency as tuned"
        );
        // One Measured per frequency, in order — the frontend maps step
        // results to frequencies positionally.
        assert_eq!(measured_count(&events), freqs.len());

        let measured: Vec<ScanEvent> = events
            .iter()
            .copied()
            .filter(|e| matches!(e, ScanEvent::Measured { .. }))
            .collect();
        assert_eq!(
            measured[1],
            ScanEvent::Measured {
                signal_avg_db: f32::NEG_INFINITY,
                noise_floor_db: f32::NEG_INFINITY
            },
            "the failed step must leave a placeholder in its own slot"
        );
        assert!(
            events.contains(&ScanEvent::Tuned {
                frequency_hz: freqs[2]
            }),
            "the sweep must continue past a failed retune"
        );
        assert_eq!(events.last(), Some(&ScanEvent::Complete));
    }

    #[tokio::test]
    async fn squelch_stops_sweep_at_first_channel_over_threshold() {
        let freqs = vec![100_000_000_u32, 100_200_000, 100_400_000];
        let tuner = FakeTuner::new(SNAP_HZ);
        let acc = accumulator();
        // A strong carrier sits in the target channel: SNR is 40 dB, over the
        // 20 dB gate, so the sweep must stop on the very first step.
        spawn_dsp_filler(acc.clone(), true);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();

        run_scanner(
            &tuner,
            LO_OFFSET_HZ,
            freqs.clone(),
            DWELL_MS,
            Some(20.0),
            acc,
            Arc::new(AtomicBool::new(false)),
            FS_HZ,
            STEP_HZ,
            move |e| sink.lock().unwrap().push(e),
        )
        .await;

        let events = events.lock().unwrap().clone();
        assert_eq!(tuner.tuned_hz(), vec![freqs[0] - LO_OFFSET_HZ]);
        assert_eq!(
            events.last(),
            Some(&ScanEvent::Stopped {
                frequency_hz: freqs[0]
            })
        );
        assert!(!events.contains(&ScanEvent::Complete));
    }

    #[tokio::test]
    async fn cancel_mid_sweep_stops_tuning_and_emits_nothing_further() {
        let freqs = vec![100_000_000_u32, 100_200_000, 100_400_000];
        let tuner = FakeTuner::new(SNAP_HZ);
        let cancel = Arc::new(AtomicBool::new(false));
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();

        let flag = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            flag.store(true, Ordering::Relaxed);
        });

        run_scanner(
            &tuner,
            LO_OFFSET_HZ,
            freqs.clone(),
            0,
            None,
            accumulator(),
            cancel,
            FS_HZ,
            STEP_HZ,
            move |e| sink.lock().unwrap().push(e),
        )
        .await;

        let events = events.lock().unwrap().clone();
        assert_eq!(
            tuner.tuned_hz(),
            vec![freqs[0] - LO_OFFSET_HZ],
            "cancellation must be observed before the next retune"
        );
        assert!(!events.contains(&ScanEvent::Complete));
        assert!(!events
            .iter()
            .any(|e| matches!(e, ScanEvent::Stopped { .. })));
    }

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
