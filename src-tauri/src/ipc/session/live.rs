//! Live RTL-SDR sessions: opening policy plus the `start_stream` command.
//!
//! Everything past the device handle is shared with replay — see
//! [`super::start::start_session`].

use std::time::Duration;

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};

use crate::dsp::demod::AUDIO_RATE_HZ;
use crate::error::RailError;
use crate::hardware::stream::{IqStream, DEFAULT_USB_BUF_LEN, DEFAULT_USB_BUF_NUM};
use crate::hardware::{RtlSdrDevice, Tuner};
use crate::ipc::control::RadioParams;
use crate::ipc::dsp_task::{AUDIO_CHUNK_SAMPLES, FFT_SIZE};
use crate::ipc::events::DeviceStatus;
use crate::ipc::session::start::{ensure_idle, start_session, ScanAccumulator, SessionPlan};
use crate::ipc::session::types::{AppState, LiveBits, SessionSource};

/// Default RTL-SDR sample rate. Stable per `docs/HARDWARE.md` §4.
const DEFAULT_SAMPLE_RATE_HZ: u32 = 2_048_000;
/// Fallback sample rates to probe if the requested one is rejected by
/// librtlsdr on a specific tuner/driver combo (`set_sample_rate -> -1`).
/// Ordered by preference.
const FALLBACK_SAMPLE_RATES_HZ: [u32; 5] = [2_048_000, 1_800_000, 1_400_000, 1_024_000, 900_000];

/// LO offset used to push the RTL-SDR DC spike off the center bin.
/// See `docs/DSP.md` §1 and the `fs/4` mixer in
/// [`crate::dsp::waterfall::apply_fs4_shift`].
pub(crate) fn lo_offset_hz(sample_rate_hz: u32) -> u32 {
    sample_rate_hz / 4
}

fn sample_rate_candidates(requested_hz: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(FALLBACK_SAMPLE_RATES_HZ.len() + 1);
    out.push(requested_hz);
    for hz in FALLBACK_SAMPLE_RATES_HZ {
        if hz != requested_hz {
            out.push(hz);
        }
    }
    out
}

/// Open the first RTL-SDR and settle on a sample rate it accepts.
///
/// On Windows with WinUSB, `rtlsdr_open` can succeed while the USB
/// endpoint is still settling — the first register write then returns
/// LIBUSB_ERROR_PIPE (-9). Retry up to 3 times with a 100 ms gap; the
/// device is always ready within one retry in practice.
/// See `docs/HARDWARE.md` §6 ("rtlsdr_demod_write_reg failed with -9").
async fn open_live_device(requested_sample_rate_hz: u32) -> Result<(RtlSdrDevice, u32), RailError> {
    const OPEN_RETRIES: usize = 3;
    let mut last_error = RailError::DeviceNotFound;

    for attempt in 0..OPEN_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            log::info!(
                "RTL-SDR open retry {attempt}/{}: USB endpoint may not be ready yet",
                OPEN_RETRIES - 1
            );
        }

        let dev = match RtlSdrDevice::open(0) {
            Ok(d) => d,
            Err(e) => {
                last_error = e;
                continue;
            }
        };

        let mut found_rate: Option<u32> = None;
        for candidate_hz in sample_rate_candidates(requested_sample_rate_hz) {
            match dev.set_sample_rate(candidate_hz) {
                Ok(()) => {
                    if candidate_hz != requested_sample_rate_hz {
                        log::warn!(
                            "sample rate {} rejected; using fallback {}",
                            requested_sample_rate_hz,
                            candidate_hz
                        );
                    }
                    found_rate = Some(candidate_hz);
                    break;
                }
                Err(e) => {
                    log::debug!("set_sample_rate({candidate_hz}): {e}");
                    last_error = RailError::StreamError(format!(
                        "failed to set sample rate (requested {requested_sample_rate_hz}): {e}"
                    ));
                }
            }
        }

        if let Some(rate) = found_rate {
            return Ok((dev, rate));
        }
        // All rates failed — likely USB pipe error; drop `dev` and retry.
    }

    Err(last_error)
}

/// Parameters for [`start_stream`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartStreamArgs {
    pub frequency_hz: u32,
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
}

/// Reply for [`start_stream`]. Tells the frontend what FFT size to
/// expect on the waterfall channel and how to interpret the audio one.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartStreamReply {
    pub fft_size: usize,
    pub sample_rate_hz: u32,
    pub frequency_hz: u32,
    pub available_gains_tenths_db: Vec<i32>,
    pub audio_sample_rate_hz: u32,
    pub audio_chunk_samples: usize,
}

/// Open the first RTL-SDR, configure it, and start the IQ → FFT/demod
/// pipeline. The frontend passes two `Channel<ArrayBuffer>` handles:
/// the first carries waterfall frames (float32), the second carries
/// mono f32 PCM audio at `audio_sample_rate_hz`.
#[tauri::command]
pub async fn start_stream<R: Runtime>(
    app: AppHandle<R>,
    args: StartStreamArgs,
    waterfall_channel: Channel<InvokeResponseBody>,
    audio_channel: Channel<InvokeResponseBody>,
    state: State<'_, AppState>,
) -> Result<StartStreamReply, RailError> {
    // The guard is a temporary of this call, so it is released before
    // the awaits below.
    ensure_idle(&state, "stream already running")?;

    let requested_sample_rate_hz = args.sample_rate_hz.unwrap_or(DEFAULT_SAMPLE_RATE_HZ);
    let (device, sample_rate) = open_live_device(requested_sample_rate_hz).await?;

    let offset = lo_offset_hz(sample_rate);
    // Park the LO `fs/4` below the user's target; the `−fs/4` digital
    // mixer in `apply_fs4_shift` brings the tuned carrier back to DC
    // with the hardware DC spike off-center (docs/DSP.md §1).
    let tuner = device.tuner();
    tuner.set_center_freq(args.frequency_hz.saturating_sub(offset))?;
    tuner.set_tuner_gain_mode(false)?;
    let gains = device.available_gains().unwrap_or_default();
    let actual_freq = tuner.center_freq().saturating_add(offset);

    let disconnect_app = app.clone();
    let attach_gains = gains.clone();
    start_session(
        &app,
        &state,
        SessionPlan {
            sample_rate_hz: sample_rate,
            params: RadioParams {
                center_hz: actual_freq,
                ..RadioParams::default()
            },
            scan_accumulator: ScanAccumulator::Sized,
        },
        waterfall_channel,
        audio_channel,
        move |iq_tx| {
            // Fires from the reader thread if the dongle is unplugged mid-stream.
            let on_disconnect: Box<dyn FnOnce(String) + Send + 'static> = Box::new(move |reason| {
                log::warn!("RTL-SDR disconnected mid-stream: {reason}");
                let _ = DeviceStatus::disconnected_with(reason).emit(&disconnect_app);
            });
            let stream = IqStream::start(
                device,
                iq_tx,
                DEFAULT_USB_BUF_NUM,
                DEFAULT_USB_BUF_LEN,
                on_disconnect,
            )?;
            let canceler = stream.canceler();
            Ok((
                SessionSource::Live(LiveBits {
                    stream: Some(stream),
                    tuner: Some(tuner),
                    gains: attach_gains,
                }),
                Some(canceler),
            ))
        },
    )?;

    Ok(StartStreamReply {
        fft_size: FFT_SIZE,
        sample_rate_hz: sample_rate,
        frequency_hz: actual_freq,
        available_gains_tenths_db: gains,
        audio_sample_rate_hz: AUDIO_RATE_HZ as u32,
        audio_chunk_samples: AUDIO_CHUNK_SAMPLES,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::sample_rate_candidates;

    #[test]
    fn sample_rate_candidates_keep_requested_first() {
        let c = sample_rate_candidates(2_400_000);
        assert_eq!(c[0], 2_400_000);
        assert!(c.contains(&2_048_000));
    }

    #[test]
    fn sample_rate_candidates_dedup_requested_rate() {
        let c = sample_rate_candidates(2_048_000);
        let count = c.iter().filter(|&&hz| hz == 2_048_000).count();
        assert_eq!(count, 1);
    }
}
