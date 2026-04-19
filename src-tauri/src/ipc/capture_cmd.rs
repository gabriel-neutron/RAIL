//! Capture commands (menu-driven audio / IQ / screenshot flows).
//!
//! These commands use a hybrid temp-file + native-dialog flow:
//! `start_*_capture` opens a temp file via
//! [`crate::capture::tmp::new_tmp_path`], the DSP worker writes into
//! it, then `finalize_*` / `discard_*` atomically moves or deletes
//! the temp file when the user confirms the Save dialog. See
//! `docs/SIGNALS.md` §1–2 and REVIEW_V1.md §5.3 for the split
//! rationale.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Runtime, State};
use tokio::sync::{mpsc, oneshot};

use crate::capture::sigmf::SigMfStartParams;
use crate::capture::tmp::{move_file, new_tmp_path};
use crate::dsp::demod::AUDIO_RATE_HZ;
use crate::error::RailError;
use crate::ipc::commands::{session_poisoned, AppState};

/// Requests from Tauri commands to the DSP worker that interact with
/// capture writers. Replies ride on a `oneshot` so commands remain
/// `async` and do not touch the DSP mutex directly.
pub(crate) enum CaptureControl {
    /// Open a [`WavStreamWriter`] at `path` and start appending the
    /// audio samples the demod produces every iteration.
    StartAudio {
        path: PathBuf,
        sample_rate_hz: u32,
        reply: oneshot::Sender<Result<(), RailError>>,
    },
    /// Close the running audio writer and return the total sample
    /// count (for a duration estimate in the final filename).
    StopAudio {
        reply: oneshot::Sender<Result<AudioStopInfo, RailError>>,
    },
    /// Open a [`SigMfStreamWriter`] and start appending the already-
    /// shifted cf32 samples (same buffer the waterfall FFT uses).
    StartIq {
        meta_path: PathBuf,
        data_path: PathBuf,
        params: SigMfStartParams,
        reply: oneshot::Sender<Result<(), RailError>>,
    },
    /// Close the running IQ writer and return the sample count.
    StopIq {
        reply: oneshot::Sender<Result<IqStopInfo, RailError>>,
    },
}

/// Per-recording stop result handed back to the Tauri command.
#[derive(Debug)]
pub(crate) struct AudioStopInfo {
    pub path: PathBuf,
    pub samples: u64,
    pub sample_rate_hz: u32,
}

#[derive(Debug)]
pub(crate) struct IqStopInfo {
    pub meta_path: PathBuf,
    pub data_path: PathBuf,
    pub samples: u64,
    pub sample_rate_hz: u32,
}

struct RadioSnapshot {
    frequency_hz: u64,
    mode: String,
    bandwidth_hz: u32,
    gain_tenths_db: Option<i32>,
    sample_rate_hz: u32,
}

fn radio_snapshot(state: &State<'_, AppState>) -> Result<RadioSnapshot, RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let s = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    Ok(RadioSnapshot {
        frequency_hz: s.frequency_hz as u64,
        mode: s.mode.clone(),
        bandwidth_hz: s.bandwidth_hz,
        gain_tenths_db: s.gain_tenths_db,
        sample_rate_hz: s.sample_rate_hz,
    })
}

fn capture_sender(
    state: &State<'_, AppState>,
) -> Result<mpsc::UnboundedSender<CaptureControl>, RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let s = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    Ok(s.capture_tx.clone())
}

/// ISO 8601 UTC "YYYYMMDDTHHMMSSZ" — no separators so it's safe in
/// filenames across OSes. Second resolution is enough for capture IDs.
fn iso8601_compact(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let secs_of_day = (epoch_secs % 86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    let h = secs_of_day / 3_600;
    let mi = (secs_of_day % 3_600) / 60;
    let se = secs_of_day % 60;
    format!("{y:04}{m:02}{d:02}T{h:02}{mi:02}{se:02}Z")
}

/// "Gregorian" date from Unix-epoch day count (Howard Hinnant's
/// `civil_from_days`). Avoids pulling in `chrono`/`time`.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + (era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn now_secs() -> Result<u64, RailError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| RailError::CaptureError(format!("clock error: {e}")))
}

/// Build a suggested filename: `RAIL_<freq>_<iso>.<ext>`. Frequency is
/// rendered with three decimal digits in MHz because the UI frequency
/// entry resolves to 1 kHz.
fn suggested_name(frequency_hz: u64, ext: &str) -> Result<String, RailError> {
    let mhz = frequency_hz as f64 / 1_000_000.0;
    let iso = iso8601_compact(now_secs()?);
    Ok(format!("RAIL_{mhz:.3}MHz_{iso}.{ext}"))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAudioCaptureReply {
    pub temp_path: String,
    pub suggested_name: String,
}

/// Open a temp-file WAV writer and start appending the demodulator's
/// 44.1 kHz f32 output to it. The temp path is handed back so the
/// frontend can pass it to `finalize_capture` (or `discard_capture`).
#[tauri::command]
pub async fn start_audio_capture<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<StartAudioCaptureReply, RailError> {
    let radio = radio_snapshot(&state)?;
    let temp = new_tmp_path(&app, "wav")?;
    let tx = capture_sender(&state)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(CaptureControl::StartAudio {
        path: temp.clone(),
        sample_rate_hz: AUDIO_RATE_HZ as u32,
        reply: reply_tx,
    })
    .map_err(|e| RailError::StreamError(format!("capture channel closed: {e}")))?;
    reply_rx
        .await
        .map_err(|e| RailError::StreamError(format!("capture reply dropped: {e}")))??;

    Ok(StartAudioCaptureReply {
        temp_path: temp.to_string_lossy().into_owned(),
        suggested_name: suggested_name(radio.frequency_hz, "wav")?,
    })
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopAudioCaptureReply {
    pub temp_path: String,
    pub suggested_name: String,
    pub frequency_hz: u64,
    pub mode: String,
    pub duration_ms: u64,
}

/// Finalize the running WAV temp file (patch its header) and return
/// the path + suggested filename so the frontend can raise a Save As.
#[tauri::command]
pub async fn stop_audio_capture(
    state: State<'_, AppState>,
) -> Result<StopAudioCaptureReply, RailError> {
    let radio = radio_snapshot(&state)?;
    let tx = capture_sender(&state)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(CaptureControl::StopAudio { reply: reply_tx })
        .map_err(|e| RailError::StreamError(format!("capture channel closed: {e}")))?;
    let info = reply_rx
        .await
        .map_err(|e| RailError::StreamError(format!("capture reply dropped: {e}")))??;

    let duration_ms = if info.sample_rate_hz > 0 {
        info.samples.saturating_mul(1_000) / info.sample_rate_hz as u64
    } else {
        0
    };
    Ok(StopAudioCaptureReply {
        temp_path: info.path.to_string_lossy().into_owned(),
        suggested_name: suggested_name(radio.frequency_hz, "wav")?,
        frequency_hz: radio.frequency_hz,
        mode: radio.mode,
        duration_ms,
    })
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartIqCaptureReply {
    pub temp_meta_path: String,
    pub temp_data_path: String,
    pub suggested_name: String,
}

/// Open a temp SigMF writer and start mirroring every shifted cf32
/// sample into it. The `.sigmf-data` path is what users care about
/// when picking a save location; the `.sigmf-meta` sibling is kept
/// alongside it automatically.
#[tauri::command]
pub async fn start_iq_capture<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<StartIqCaptureReply, RailError> {
    let radio = radio_snapshot(&state)?;
    let data_temp = new_tmp_path(&app, "sigmf-data")?;
    let meta_temp = data_temp.with_extension("sigmf-meta");
    let tx = capture_sender(&state)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    let params = SigMfStartParams {
        sample_rate_hz: radio.sample_rate_hz,
        center_frequency_hz: radio.frequency_hz,
        tuner_gain_db: radio
            .gain_tenths_db
            .map(|t| t as f32 / 10.0)
            .unwrap_or(f32::NAN),
        demod_mode: radio.mode.clone(),
        filter_bandwidth_hz: radio.bandwidth_hz,
        datetime_iso8601: iso8601_compact(now_secs()?),
    };
    tx.send(CaptureControl::StartIq {
        meta_path: meta_temp.clone(),
        data_path: data_temp.clone(),
        params,
        reply: reply_tx,
    })
    .map_err(|e| RailError::StreamError(format!("capture channel closed: {e}")))?;
    reply_rx
        .await
        .map_err(|e| RailError::StreamError(format!("capture reply dropped: {e}")))??;

    Ok(StartIqCaptureReply {
        temp_meta_path: meta_temp.to_string_lossy().into_owned(),
        temp_data_path: data_temp.to_string_lossy().into_owned(),
        suggested_name: suggested_name(radio.frequency_hz, "sigmf-data")?,
    })
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopIqCaptureReply {
    pub temp_meta_path: String,
    pub temp_data_path: String,
    pub suggested_name: String,
    pub frequency_hz: u64,
    pub duration_ms: u64,
}

/// Finalize the running SigMF temp files and return the pair + a
/// suggested filename for the save dialog.
#[tauri::command]
pub async fn stop_iq_capture(state: State<'_, AppState>) -> Result<StopIqCaptureReply, RailError> {
    let radio = radio_snapshot(&state)?;
    let tx = capture_sender(&state)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(CaptureControl::StopIq { reply: reply_tx })
        .map_err(|e| RailError::StreamError(format!("capture channel closed: {e}")))?;
    let info = reply_rx
        .await
        .map_err(|e| RailError::StreamError(format!("capture reply dropped: {e}")))??;

    let duration_ms = if info.sample_rate_hz > 0 {
        info.samples.saturating_mul(1_000) / info.sample_rate_hz as u64
    } else {
        0
    };
    Ok(StopIqCaptureReply {
        temp_meta_path: info.meta_path.to_string_lossy().into_owned(),
        temp_data_path: info.data_path.to_string_lossy().into_owned(),
        suggested_name: suggested_name(radio.frequency_hz, "sigmf-data")?,
        frequency_hz: radio.frequency_hz,
        duration_ms,
    })
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizeCaptureArgs {
    pub src: String,
    pub dst: String,
}

/// Move a single temp file to the user-chosen destination (WAV, PNG).
#[tauri::command]
pub fn finalize_capture(args: FinalizeCaptureArgs) -> Result<(), RailError> {
    move_file(&PathBuf::from(args.src), &PathBuf::from(args.dst))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizeIqCaptureArgs {
    pub src_meta: String,
    pub src_data: String,
    pub dst_meta: String,
    pub dst_data: String,
}

/// Move the SigMF pair (`.sigmf-meta` + `.sigmf-data`) to the user-
/// chosen destination. The frontend derives `dstMeta` from `dstData`
/// by swapping the extension so the pair stays together.
#[tauri::command]
pub fn finalize_iq_capture(args: FinalizeIqCaptureArgs) -> Result<(), RailError> {
    move_file(&PathBuf::from(args.src_data), &PathBuf::from(args.dst_data))?;
    move_file(&PathBuf::from(args.src_meta), &PathBuf::from(args.dst_meta))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardCaptureArgs {
    pub paths: Vec<String>,
}

/// Unlink any temp files the frontend was about to finalize. Used when
/// the user cancels the Save dialog. Missing files are ignored.
#[tauri::command]
pub fn discard_capture(args: DiscardCaptureArgs) -> Result<(), RailError> {
    for p in args.paths {
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveScreenshotArgs {
    pub dst: String,
    pub png_bytes: Vec<u8>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotSuggestionReply {
    pub suggested_name: String,
}

/// Build a suggested filename for the waterfall PNG using the current
/// tuned frequency. Paired with [`save_screenshot`]: the frontend
/// calls this first to populate the Save dialog.
#[tauri::command]
pub fn screenshot_suggestion(
    state: State<'_, AppState>,
) -> Result<ScreenshotSuggestionReply, RailError> {
    let radio = radio_snapshot(&state)?;
    Ok(ScreenshotSuggestionReply {
        suggested_name: suggested_name(radio.frequency_hz, "png")?,
    })
}

/// Write the PNG bytes supplied by the frontend (usually from
/// `canvas.toBlob('image/png')`) to the user-chosen destination.
#[tauri::command]
pub fn save_screenshot(args: SaveScreenshotArgs) -> Result<(), RailError> {
    if args.png_bytes.len() < 8 || &args.png_bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(RailError::CaptureError("expected PNG bytes".into()));
    }
    let dst = PathBuf::from(args.dst);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RailError::CaptureError(format!("screenshot dir: {e}")))?;
    }
    let tmp = dst.with_extension("png.tmp");
    std::fs::write(&tmp, &args.png_bytes)
        .map_err(|e| RailError::CaptureError(format!("png write: {e}")))?;
    std::fs::rename(&tmp, &dst).map_err(|e| RailError::CaptureError(format!("png rename: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_is_20240101t000000z_at_epoch_plus_one_day() {
        assert_eq!(iso8601_compact(86_400), "19700102T000000Z");
    }

    #[test]
    fn iso8601_matches_2024_01_02_12_34_56() {
        // 2024-01-02T12:34:56Z = 1704198896
        assert_eq!(iso8601_compact(1_704_198_896), "20240102T123456Z");
    }

    #[test]
    fn suggested_name_is_formatted_as_expected() {
        let name = suggested_name(100_100_000, "wav").unwrap();
        assert!(name.starts_with("RAIL_100.100MHz_"));
        assert!(name.ends_with(".wav"));
    }
}
