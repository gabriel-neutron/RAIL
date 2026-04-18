//! Tauri command handlers (React → Rust).
//!
//! Streaming data flows back to the frontend through two per-session
//! `Channel<InvokeResponseBody>`s that the frontend passes to
//! [`start_stream`]: one for waterfall frames, one for f32 PCM audio.
//! See `docs/ARCHITECTURE.md` §3 and `docs/DSP.md` §4–5.
//!
//! Capture (screenshot / audio / IQ) is menu-driven and goes through a
//! hybrid temp-file + native-dialog flow — see `docs/SIGNALS.md` §1–2
//! and the commands at the bottom of this file.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytemuck::cast_slice;
use num_complex::Complex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::{mpsc, oneshot};

use crate::bookmarks::{Bookmark, BookmarksStore};
use crate::capture::sigmf::{SigMfStartParams, SigMfStreamWriter};
use crate::capture::tmp::{move_file, new_tmp_path};
use crate::capture::wav::WavStreamWriter;
use crate::dsp::demod::{DemodChain, DemodControl, DemodMode, AUDIO_RATE_HZ};
use crate::dsp::input::DspInput;
use crate::dsp::waterfall::{apply_fs4_shift, iq_u8_to_complex, FrameBuilder};
use crate::error::RailError;
use crate::hardware::stream::{
    IqCanceler, IqStream, DEFAULT_USB_BUF_LEN, DEFAULT_USB_BUF_NUM, IQ_CHANNEL_CAPACITY,
};
use crate::hardware::{self, DeviceInfo, RtlSdrDevice, TunerHandle};
use crate::ipc::events::{DeviceStatus, SignalLevel};
use crate::replay::{spawn_replay_reader, ReplayControl, ReplayInfo};

/// FFT size (bins). Matches `docs/DSP.md` §2 default.
pub(crate) const FFT_SIZE: usize = 2048;

/// Default RTL-SDR sample rate. Stable per `docs/HARDWARE.md` §4.
const DEFAULT_SAMPLE_RATE_HZ: u32 = 2_048_000;

/// Minimum interval between waterfall frames emitted to the frontend
/// (~25 fps cap, `docs/DSP.md` §3).
const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(40);

/// Minimum interval between `signal-level` JSON events (~25 Hz).
/// Same cadence as waterfall frames — keeps meter and spectrum in step.
const MIN_LEVEL_EMIT_INTERVAL: Duration = Duration::from_millis(40);

/// Decay per emission for the backend peak-hold used by the signal
/// meter. 1 dB/frame at ~25 Hz gives a ~4 s fall from a peak to the
/// noise floor, which reads naturally in the UI.
const PEAK_DECAY_DB_PER_EMIT: f32 = 1.0;

/// Target audio chunk length emitted to the frontend (~40 ms @ 44.1 kHz).
/// The resampler output is variable, so this is a *minimum* drain size.
const AUDIO_CHUNK_SAMPLES: usize = 1764;

/// Mode names accepted over the wire. Kept in sync with
/// `src/store/radio.ts :: DemodMode`.
fn parse_mode(s: &str) -> Result<DemodMode, RailError> {
    match s {
        "FM" => Ok(DemodMode::Fm),
        "AM" => Ok(DemodMode::Am),
        "USB" | "LSB" | "CW" => Err(RailError::InvalidParameter(format!(
            "{s} demodulator is stubbed for V1.1 (see docs/DSP.md §6)"
        ))),
        other => Err(RailError::InvalidParameter(format!(
            "unknown mode: {other}"
        ))),
    }
}

/// One running streaming session. Held inside [`AppState`].
///
/// A session is either *live* (RTL-SDR reader + tuner hardware) or
/// *replay* (SigMF file reader). The DSP-facing fields (`dsp`,
/// `control_tx`, `capture_tx`, radio snapshot) are shared so the
/// demod-control and capture commands don't care which source is
/// running. The [`source`](Session::source) enum only covers the
/// bits that differ between the two modes.
struct Session {
    /// JoinHandle for the DSP task (stops when the IQ sender drops).
    dsp: Option<tokio::task::JoinHandle<()>>,
    /// Sample rate of the IQ stream feeding the DSP task.
    sample_rate_hz: u32,
    /// Outbound channel for runtime demod control (mode/bandwidth/squelch).
    control_tx: mpsc::UnboundedSender<DemodControl>,
    /// Outbound channel for capture-related requests (audio / IQ).
    capture_tx: mpsc::UnboundedSender<CaptureControl>,
    /// Most recent centre frequency, kept in sync with `retune` so the
    /// capture metadata and suggested filenames always match what the
    /// user sees.
    frequency_hz: u32,
    /// Most recent demod mode (`FM` / `AM`), updated on `set_mode`.
    mode: String,
    /// Most recent channel bandwidth (Hz), updated on `set_bandwidth`.
    bandwidth_hz: u32,
    /// Most recent manual gain (tenths of dB). `None` while in AGC.
    gain_tenths_db: Option<i32>,
    /// Source-specific bits (live hardware vs replay file).
    source: SessionSource,
}

/// Source-specific state for a [`Session`].
enum SessionSource {
    Live(LiveBits),
    Replay(ReplayBits),
}

struct LiveBits {
    /// RAII for the reader thread. Option so we can take it out in `stop`.
    stream: Option<IqStream>,
    /// Thread-safe tuning surface. `None` after the session is torn down
    /// so that late `set_gain` calls return an error instead of racing
    /// with device close.
    tuner: Option<TunerHandle>,
    /// Discrete gain steps the hardware supports (tenths of dB).
    gains: Vec<i32>,
}

struct ReplayBits {
    /// JoinHandle for the replay reader task.
    reader: Option<tokio::task::JoinHandle<()>>,
    /// Transport control channel (play/pause/seek/stop).
    control_tx: mpsc::UnboundedSender<ReplayControl>,
    /// Cached file metadata — handed back to the frontend on open /
    /// used to clamp seek positions without re-reading the file.
    info: ReplayInfo,
}

/// Requests from Tauri commands to the DSP worker that interact with
/// capture writers. Replies ride on a `oneshot` so commands remain
/// `async` and do not touch the DSP mutex directly.
enum CaptureControl {
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
struct AudioStopInfo {
    path: PathBuf,
    samples: u64,
    sample_rate_hz: u32,
}

#[derive(Debug)]
struct IqStopInfo {
    meta_path: PathBuf,
    data_path: PathBuf,
    samples: u64,
    sample_rate_hz: u32,
}

/// LO offset used to push the RTL-SDR DC spike off the center bin.
/// See `docs/DSP.md` §1 and the `fs/4` mixer in
/// [`crate::dsp::waterfall::apply_fs4_shift`].
fn lo_offset_hz(sample_rate_hz: u32) -> u32 {
    sample_rate_hz / 4
}

/// Global, single-session state.
#[derive(Default)]
pub struct AppState {
    session: Mutex<Option<Session>>,
}

/// Liveness check: returns `"pong"`. Used by the frontend on startup to
/// verify the IPC bridge is healthy.
#[tauri::command]
pub fn ping() -> &'static str {
    "pong"
}

/// Enumerate attached RTL-SDR compatible USB devices via `nusb`.
/// Returns the first match or `RailError::DeviceNotFound`.
#[tauri::command]
pub fn check_device() -> Result<DeviceInfo, RailError> {
    hardware::check_device()
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
    let mut guard = state.session.lock().map_err(session_poisoned)?;
    if guard.is_some() {
        return Err(RailError::InvalidParameter(
            "stream already running".into(),
        ));
    }

    let sample_rate = args.sample_rate_hz.unwrap_or(DEFAULT_SAMPLE_RATE_HZ);
    let offset = lo_offset_hz(sample_rate);

    let device = RtlSdrDevice::open(0)?;
    device.set_sample_rate(sample_rate)?;
    // Park the LO `fs/4` above the user's target so the DC spike sits
    // off-center after the DSP mixer (docs/DSP.md §1).
    device.set_center_freq(args.frequency_hz.saturating_add(offset))?;
    device.set_tuner_gain_mode(false)?;
    let gains = device.available_gains().unwrap_or_default();

    let tuner = device.tuner_handle();
    let actual_freq = device.center_freq().saturating_sub(offset);

    let (iq_tx, iq_rx) = mpsc::channel::<DspInput>(IQ_CHANNEL_CAPACITY);
    let (control_tx, control_rx) = mpsc::unbounded_channel::<DemodControl>();
    let (capture_tx, capture_rx) = mpsc::unbounded_channel::<CaptureControl>();

    // Fires from the reader thread if the dongle is unplugged mid-stream.
    let disconnect_app = app.clone();
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

    let dsp_handle = spawn_dsp_task(
        app.clone(),
        iq_rx,
        waterfall_channel,
        audio_channel,
        control_rx,
        capture_rx,
        Some(canceler),
        sample_rate,
    );

    *guard = Some(Session {
        dsp: Some(dsp_handle),
        sample_rate_hz: sample_rate,
        control_tx,
        capture_tx,
        frequency_hz: actual_freq,
        mode: "FM".into(),
        bandwidth_hz: 200_000,
        gain_tenths_db: None,
        source: SessionSource::Live(LiveBits {
            stream: Some(stream),
            tuner: Some(tuner),
            gains: gains.clone(),
        }),
    });
    drop(guard);

    let _ = DeviceStatus::connected().emit(&app);

    Ok(StartStreamReply {
        fft_size: FFT_SIZE,
        sample_rate_hz: sample_rate,
        frequency_hz: actual_freq,
        available_gains_tenths_db: gains,
        audio_sample_rate_hz: AUDIO_RATE_HZ as u32,
        audio_chunk_samples: AUDIO_CHUNK_SAMPLES,
    })
}

/// Stop the streaming session and release the hardware. Idempotent.
///
/// The `_app` is kept in the signature so `stop_replay` can forward
/// its own `AppHandle` here without an extra shim. No device-status
/// event is emitted — `stop_stream` is always an intentional
/// frontend-initiated teardown, so the caller already knows the
/// stream ended (see the `DeviceStatus::disconnected_with` path in
/// `on_disconnect` for the genuine-disconnect case).
#[tauri::command]
pub async fn stop_stream<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), RailError> {
    let session = {
        let mut guard = state.session.lock().map_err(session_poisoned)?;
        guard.take()
    };
    let Some(mut session) = session else {
        return Ok(());
    };

    let shutdown_result = match &mut session.source {
        SessionSource::Live(live) => {
            live.tuner.take();
            live.stream.take().map(|s| s.stop()).unwrap_or(Ok(()))
        }
        SessionSource::Replay(replay) => {
            let _ = replay.control_tx.send(ReplayControl::Stop);
            if let Some(reader) = replay.reader.take() {
                let _ = reader.await;
            }
            Ok(())
        }
    };
    if let Some(dsp) = session.dsp.take() {
        let _ = dsp.await;
    }

    shutdown_result
}

/// Arguments for [`set_gain`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetGainArgs {
    pub auto: bool,
    #[serde(default)]
    pub tenths_db: Option<i32>,
}

#[tauri::command]
pub fn set_gain(args: SetGainArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    let mut guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_mut()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    let live = match &mut session.source {
        SessionSource::Live(l) => l,
        SessionSource::Replay(_) => {
            return Err(RailError::InvalidParameter(
                "gain cannot be changed during replay".into(),
            ))
        }
    };
    let tuner = live
        .tuner
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("tuner unavailable".into()))?;

    tuner.set_tuner_gain_mode(!args.auto)?;
    if args.auto {
        session.gain_tenths_db = None;
    } else {
        let tenths = args
            .tenths_db
            .ok_or_else(|| RailError::InvalidParameter("manual gain requires tenthsDb".into()))?;
        if !live.gains.is_empty() && !live.gains.contains(&tenths) {
            return Err(RailError::InvalidParameter(format!(
                "gain {tenths} not in supported set"
            )));
        }
        tuner.set_tuner_gain_tenths(tenths)?;
        session.gain_tenths_db = Some(tenths);
    }
    Ok(())
}

#[tauri::command]
pub fn available_gains(state: State<'_, AppState>) -> Result<Vec<i32>, RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    Ok(guard
        .as_ref()
        .and_then(|s| match &s.source {
            SessionSource::Live(l) => Some(l.gains.clone()),
            SessionSource::Replay(_) => None,
        })
        .unwrap_or_default())
}

/// Arguments for [`retune`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetuneArgs {
    pub frequency_hz: u32,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetuneReply {
    pub frequency_hz: u32,
}

#[tauri::command]
pub fn retune(args: RetuneArgs, state: State<'_, AppState>) -> Result<RetuneReply, RailError> {
    let mut guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_mut()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    let live = match &mut session.source {
        SessionSource::Live(l) => l,
        SessionSource::Replay(_) => {
            return Err(RailError::InvalidParameter(
                "retune is not supported during replay".into(),
            ))
        }
    };
    let tuner = live
        .tuner
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("tuner unavailable".into()))?;

    let offset = lo_offset_hz(session.sample_rate_hz);
    tuner.set_center_freq(args.frequency_hz.saturating_add(offset))?;
    let freq = tuner.center_freq().saturating_sub(offset);
    session.frequency_hz = freq;
    Ok(RetuneReply {
        frequency_hz: freq,
    })
}

/// Arguments for [`set_ppm`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPpmArgs {
    pub ppm: i32,
}

#[tauri::command]
pub fn set_ppm(args: SetPpmArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    let live = match &session.source {
        SessionSource::Live(l) => l,
        SessionSource::Replay(_) => {
            return Err(RailError::InvalidParameter(
                "PPM correction is not available during replay".into(),
            ))
        }
    };
    let tuner = live
        .tuner
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("tuner unavailable".into()))?;

    tuner.set_freq_correction_ppm(args.ppm)
}

/// Arguments for [`set_mode`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetModeArgs {
    pub mode: String,
}

#[tauri::command]
pub fn set_mode(args: SetModeArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    let mode = parse_mode(&args.mode)?;
    {
        let mut guard = state.session.lock().map_err(session_poisoned)?;
        if let Some(s) = guard.as_mut() {
            s.mode = match mode {
                DemodMode::Fm => "FM".into(),
                DemodMode::Am => "AM".into(),
            };
        }
    }
    send_control(&state, DemodControl::SetMode(mode))
}

/// Arguments for [`set_bandwidth`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBandwidthArgs {
    pub bandwidth_hz: u32,
}

#[tauri::command]
pub fn set_bandwidth(
    args: SetBandwidthArgs,
    state: State<'_, AppState>,
) -> Result<(), RailError> {
    if args.bandwidth_hz < 1_000 {
        return Err(RailError::InvalidParameter(
            "bandwidth must be >= 1 kHz".into(),
        ));
    }
    {
        let mut guard = state.session.lock().map_err(session_poisoned)?;
        if let Some(s) = guard.as_mut() {
            s.bandwidth_hz = args.bandwidth_hz;
        }
    }
    send_control(
        &state,
        DemodControl::SetBandwidthHz(args.bandwidth_hz as f32),
    )
}

/// Arguments for [`set_squelch`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSquelchArgs {
    pub threshold_dbfs: Option<f32>,
}

#[tauri::command]
pub fn set_squelch(
    args: SetSquelchArgs,
    state: State<'_, AppState>,
) -> Result<(), RailError> {
    let db = args
        .threshold_dbfs
        .filter(|v| v.is_finite())
        .unwrap_or(f32::NEG_INFINITY);
    send_control(&state, DemodControl::SetSquelchDbfs(db))
}

fn send_control(
    state: &State<'_, AppState>,
    msg: DemodControl,
) -> Result<(), RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    session
        .control_tx
        .send(msg)
        .map_err(|e| RailError::StreamError(format!("demod control channel closed: {e}")))
}

/// Arguments for [`add_bookmark`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddBookmarkArgs {
    pub name: String,
    pub frequency_hz: u32,
}

#[tauri::command]
pub fn list_bookmarks<R: Runtime>(
    app: AppHandle<R>,
    store: State<'_, BookmarksStore>,
) -> Result<Vec<Bookmark>, RailError> {
    store.list(&app)
}

#[tauri::command]
pub fn add_bookmark<R: Runtime>(
    app: AppHandle<R>,
    args: AddBookmarkArgs,
    store: State<'_, BookmarksStore>,
) -> Result<Bookmark, RailError> {
    store.add(&app, args.name, args.frequency_hz)
}

/// Arguments for [`remove_bookmark`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveBookmarkArgs {
    pub id: String,
}

#[tauri::command]
pub fn remove_bookmark<R: Runtime>(
    app: AppHandle<R>,
    args: RemoveBookmarkArgs,
    store: State<'_, BookmarksStore>,
) -> Result<(), RailError> {
    store.remove(&app, &args.id)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceBookmarksArgs {
    pub bookmarks: Vec<Bookmark>,
}

#[tauri::command]
pub fn replace_bookmarks<R: Runtime>(
    app: AppHandle<R>,
    args: ReplaceBookmarksArgs,
    store: State<'_, BookmarksStore>,
) -> Result<Vec<Bookmark>, RailError> {
    store.replace(&app, args.bookmarks)
}

fn session_poisoned<T>(_: std::sync::PoisonError<T>) -> RailError {
    RailError::StreamError("session lock poisoned".into())
}

// ------------------------------------------------------------------
// Capture commands (menu-driven, hybrid temp-file + native dialog)
// ------------------------------------------------------------------

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
pub async fn stop_iq_capture(
    state: State<'_, AppState>,
) -> Result<StopIqCaptureReply, RailError> {
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
pub fn screenshot_suggestion(state: State<'_, AppState>) -> Result<ScreenshotSuggestionReply, RailError> {
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
    std::fs::rename(&tmp, &dst)
        .map_err(|e| RailError::CaptureError(format!("png rename: {e}")))?;
    Ok(())
}

// ------------------------------------------------------------------
// Replay commands (IQ file playback)
// ------------------------------------------------------------------

/// Serializable snapshot of [`ReplayInfo`] handed back to the frontend.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayInfoReply {
    pub data_path: String,
    pub meta_path: String,
    pub sample_rate_hz: u32,
    pub center_frequency_hz: u64,
    pub demod_mode: String,
    pub filter_bandwidth_hz: u32,
    pub total_samples: u64,
    pub duration_ms: u64,
    pub datetime_iso8601: String,
}

impl ReplayInfoReply {
    fn from_info(info: &ReplayInfo) -> Self {
        Self {
            data_path: info.data_path.to_string_lossy().into_owned(),
            meta_path: info.meta_path.to_string_lossy().into_owned(),
            sample_rate_hz: info.sample_rate_hz,
            center_frequency_hz: info.center_frequency_hz,
            demod_mode: info.demod_mode.clone(),
            filter_bandwidth_hz: info.filter_bandwidth_hz,
            total_samples: info.total_samples,
            duration_ms: info.duration_ms(),
            datetime_iso8601: info.datetime_iso8601.clone(),
        }
    }
}

/// Inspect a `.sigmf-data` file without opening a session. Lets the
/// frontend populate the transport UI (duration, sample rate, centre
/// frequency) before it commits to starting replay.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenReplayArgs {
    pub data_path: String,
}

#[tauri::command]
pub fn open_replay(args: OpenReplayArgs) -> Result<ReplayInfoReply, RailError> {
    let info = crate::replay::load_info(std::path::Path::new(&args.data_path))?;
    Ok(ReplayInfoReply::from_info(&info))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartReplayArgs {
    pub data_path: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartReplayReply {
    pub fft_size: usize,
    pub sample_rate_hz: u32,
    pub frequency_hz: u32,
    pub audio_sample_rate_hz: u32,
    pub audio_chunk_samples: usize,
    pub info: ReplayInfoReply,
}

/// Open a SigMF `.sigmf-data` file and start the same DSP task the
/// live stream uses, but fed by [`crate::replay::spawn_replay_reader`]
/// instead of the RTL-SDR. Mirrors [`start_stream`]'s shape: the
/// caller hands in two binary channels (waterfall, audio).
#[tauri::command]
pub async fn start_replay<R: Runtime>(
    app: AppHandle<R>,
    args: StartReplayArgs,
    waterfall_channel: Channel<InvokeResponseBody>,
    audio_channel: Channel<InvokeResponseBody>,
    state: State<'_, AppState>,
) -> Result<StartReplayReply, RailError> {
    {
        let guard = state.session.lock().map_err(session_poisoned)?;
        if guard.is_some() {
            return Err(RailError::InvalidParameter(
                "stop the current stream before opening a file".into(),
            ));
        }
    }

    let info = crate::replay::load_info(std::path::Path::new(&args.data_path))?;
    let sample_rate = info.sample_rate_hz;
    let frequency_hz = u32::try_from(info.center_frequency_hz).unwrap_or(u32::MAX);
    let mode = if info.demod_mode.is_empty() {
        "FM".to_string()
    } else {
        info.demod_mode.clone()
    };
    let bandwidth_hz = if info.filter_bandwidth_hz == 0 {
        200_000
    } else {
        info.filter_bandwidth_hz
    };

    let (iq_tx, iq_rx) = mpsc::channel::<DspInput>(IQ_CHANNEL_CAPACITY);
    let (control_tx, control_rx) = mpsc::unbounded_channel::<DemodControl>();
    let (capture_tx, capture_rx) = mpsc::unbounded_channel::<CaptureControl>();
    let (replay_ctl_tx, replay_ctl_rx) = mpsc::unbounded_channel::<ReplayControl>();

    // No hardware reader to cancel — the DSP task exits cleanly when
    // the replay reader drops its `iq_tx`, and `stop_replay` sends
    // `ReplayControl::Stop` to break out of the pacing loop.
    let dsp_handle = spawn_dsp_task(
        app.clone(),
        iq_rx,
        waterfall_channel,
        audio_channel,
        control_rx,
        capture_rx,
        None,
        sample_rate,
    );

    let reader_handle = spawn_replay_reader(app.clone(), info.clone(), iq_tx, replay_ctl_rx);

    let mut guard = state.session.lock().map_err(session_poisoned)?;
    *guard = Some(Session {
        dsp: Some(dsp_handle),
        sample_rate_hz: sample_rate,
        control_tx,
        capture_tx,
        frequency_hz,
        mode,
        bandwidth_hz,
        gain_tenths_db: None,
        source: SessionSource::Replay(ReplayBits {
            reader: Some(reader_handle),
            control_tx: replay_ctl_tx,
            info: info.clone(),
        }),
    });
    drop(guard);

    let _ = DeviceStatus::connected().emit(&app);

    Ok(StartReplayReply {
        fft_size: FFT_SIZE,
        sample_rate_hz: sample_rate,
        frequency_hz,
        audio_sample_rate_hz: AUDIO_RATE_HZ as u32,
        audio_chunk_samples: AUDIO_CHUNK_SAMPLES,
        info: ReplayInfoReply::from_info(&info),
    })
}

fn replay_control_tx(
    state: &State<'_, AppState>,
) -> Result<mpsc::UnboundedSender<ReplayControl>, RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("no session running".into()))?;
    match &session.source {
        SessionSource::Replay(r) => Ok(r.control_tx.clone()),
        SessionSource::Live(_) => Err(RailError::InvalidParameter(
            "no replay in progress".into(),
        )),
    }
}

#[tauri::command]
pub fn pause_replay(state: State<'_, AppState>) -> Result<(), RailError> {
    let tx = replay_control_tx(&state)?;
    tx.send(ReplayControl::Pause)
        .map_err(|e| RailError::StreamError(format!("replay control channel closed: {e}")))
}

#[tauri::command]
pub fn resume_replay(state: State<'_, AppState>) -> Result<(), RailError> {
    let tx = replay_control_tx(&state)?;
    tx.send(ReplayControl::Play)
        .map_err(|e| RailError::StreamError(format!("replay control channel closed: {e}")))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeekReplayArgs {
    pub position_ms: u64,
}

#[tauri::command]
pub fn seek_replay(
    args: SeekReplayArgs,
    state: State<'_, AppState>,
) -> Result<(), RailError> {
    // Clamp against the cached total_samples inside the session so a
    // stale slider value can't confuse the reader.
    let (tx, sample_idx) = {
        let guard = state.session.lock().map_err(session_poisoned)?;
        let session = guard
            .as_ref()
            .ok_or_else(|| RailError::InvalidParameter("no session running".into()))?;
        match &session.source {
            SessionSource::Replay(r) => (
                r.control_tx.clone(),
                crate::replay::ms_to_sample_idx(
                    args.position_ms,
                    r.info.sample_rate_hz,
                    r.info.total_samples,
                ),
            ),
            SessionSource::Live(_) => {
                return Err(RailError::InvalidParameter(
                    "no replay in progress".into(),
                ))
            }
        }
    };
    tx.send(ReplayControl::Seek { sample_idx })
        .map_err(|e| RailError::StreamError(format!("replay control channel closed: {e}")))
}

/// Tear down a replay session. Idempotent; behaves like
/// [`stop_stream`] for a live session.
#[tauri::command]
pub async fn stop_replay<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), RailError> {
    stop_stream(app, state).await
}

// ------------------------------------------------------------------
// DSP worker
// ------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn spawn_dsp_task<R: Runtime>(
    app: AppHandle<R>,
    iq_rx: mpsc::Receiver<DspInput>,
    waterfall_channel: Channel<InvokeResponseBody>,
    audio_channel: Channel<InvokeResponseBody>,
    control_rx: mpsc::UnboundedReceiver<DemodControl>,
    capture_rx: mpsc::UnboundedReceiver<CaptureControl>,
    canceler: Option<IqCanceler>,
    sample_rate_hz: u32,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let mut ctx = DspTaskCtx::<R>::new(app, sample_rate_hz);
        ctx.run(
            iq_rx,
            waterfall_channel,
            audio_channel,
            control_rx,
            capture_rx,
            canceler,
        );
    })
}

struct DspTaskCtx<R: Runtime> {
    app: AppHandle<R>,
    builder: FrameBuilder,
    chain: DemodChain,
    /// IQ samples converted to complex, already `fs/4`-shifted.
    shifted: Vec<Complex<f32>>,
    /// Samples awaiting enough length for a full FFT frame.
    fft_pending: Vec<Complex<f32>>,
    /// Resampled audio awaiting enough length to ship a chunk.
    audio_pending: Vec<f32>,
    phase_idx: u32,
    last_emit: Instant,
    peak_dbfs: f32,
    last_level_emit: Instant,
    sample_rate_hz: u32,
    /// `Some` while an audio recording is in progress.
    audio_writer: Option<WavStreamWriter>,
    /// `Some` while an IQ recording is in progress.
    iq_writer: Option<SigMfStreamWriter>,
}

impl<R: Runtime> DspTaskCtx<R> {
    fn new(app: AppHandle<R>, sample_rate_hz: u32) -> Self {
        Self {
            app,
            builder: FrameBuilder::new(FFT_SIZE),
            chain: DemodChain::new(sample_rate_hz as f32),
            shifted: Vec::with_capacity(DEFAULT_USB_BUF_LEN as usize / 2),
            fft_pending: Vec::with_capacity(FFT_SIZE * 2),
            audio_pending: Vec::with_capacity(AUDIO_CHUNK_SAMPLES * 2),
            phase_idx: 0,
            last_emit: Instant::now() - MIN_EMIT_INTERVAL,
            peak_dbfs: f32::NEG_INFINITY,
            last_level_emit: Instant::now() - MIN_LEVEL_EMIT_INTERVAL,
            sample_rate_hz,
            audio_writer: None,
            iq_writer: None,
        }
    }

    fn run(
        &mut self,
        mut iq_rx: mpsc::Receiver<DspInput>,
        waterfall_channel: Channel<InvokeResponseBody>,
        audio_channel: Channel<InvokeResponseBody>,
        mut control_rx: mpsc::UnboundedReceiver<DemodControl>,
        mut capture_rx: mpsc::UnboundedReceiver<CaptureControl>,
        canceler: Option<IqCanceler>,
    ) {
        while let Some(input) = iq_rx.blocking_recv() {
            while let Ok(msg) = control_rx.try_recv() {
                self.chain.apply(msg);
            }
            while let Ok(msg) = capture_rx.try_recv() {
                self.handle_capture(msg);
            }

            // Prefill is a special short-circuit path: one FFT window
            // per message, waterfall-only, no audio, no rate limit.
            // Used by `crate::replay` to backfill history on seek.
            if let DspInput::Cf32Prefill(samples) = input {
                if !self.emit_prefill_frame(
                    &samples,
                    &waterfall_channel,
                    canceler.as_ref(),
                ) {
                    return;
                }
                continue;
            }

            match input {
                DspInput::RtlU8(chunk) => {
                    if chunk.len() % 2 != 0 {
                        log::warn!("discarding odd-length IQ chunk: {} bytes", chunk.len());
                        continue;
                    }
                    let n_complex = chunk.len() / 2;
                    self.shifted.resize(n_complex, Complex::new(0.0, 0.0));
                    if let Err(e) = iq_u8_to_complex(&chunk, &mut self.shifted) {
                        log::warn!("IQ conversion failed: {e}");
                        continue;
                    }
                    self.phase_idx = apply_fs4_shift(&mut self.shifted, self.phase_idx);
                }
                DspInput::Cf32Shifted(samples) => {
                    // Replay: samples come straight from a .sigmf-data
                    // file that was written downstream of the fs/4
                    // mixer. Skip both conversion and the shift — the
                    // data is already in the same form `self.shifted`
                    // would have after the RtlU8 branch runs.
                    self.shifted = samples;
                }
                DspInput::Cf32Prefill(_) => unreachable!("handled above"),
            }

            // Mirror shifted cf32 to the SigMF writer (if recording)
            // before any other fan-out — `self.shifted` is the same
            // buffer the waterfall FFT consumes, so the IQ file stays
            // phase-continuous with what the user sees.
            if let Some(w) = self.iq_writer.as_mut() {
                if let Err(e) = w.append_shifted(&self.shifted) {
                    log::warn!("iq writer failed, stopping recording: {e}");
                    self.iq_writer = None;
                }
            }

            if !self.emit_waterfall_frames(&waterfall_channel, canceler.as_ref()) {
                return;
            }

            let before = self.audio_pending.len();
            let rms_dbfs = self.chain.process(&self.shifted, &mut self.audio_pending);
            if let Some(w) = self.audio_writer.as_mut() {
                if let Err(e) = w.append(&self.audio_pending[before..]) {
                    log::warn!("audio writer failed, stopping recording: {e}");
                    self.audio_writer = None;
                }
            }
            if !self.emit_audio_chunks(&audio_channel, canceler.as_ref()) {
                return;
            }

            self.emit_signal_level(rms_dbfs);
        }

        log::debug!("dsp task exiting: iq sender dropped");
    }

    fn handle_capture(&mut self, msg: CaptureControl) {
        match msg {
            CaptureControl::StartAudio {
                path,
                sample_rate_hz,
                reply,
            } => {
                let result = if self.audio_writer.is_some() {
                    Err(RailError::CaptureError(
                        "audio recording already in progress".into(),
                    ))
                } else {
                    match WavStreamWriter::create(&path, sample_rate_hz) {
                        Ok(w) => {
                            self.audio_writer = Some(w);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                };
                let _ = reply.send(result);
            }
            CaptureControl::StopAudio { reply } => {
                let result = match self.audio_writer.take() {
                    Some(w) => {
                        let path = w.path().to_path_buf();
                        let sample_rate_hz = w.sample_rate_hz();
                        match w.finalize() {
                            Ok(samples) => Ok(AudioStopInfo {
                                path,
                                samples,
                                sample_rate_hz,
                            }),
                            Err(e) => Err(e),
                        }
                    }
                    None => Err(RailError::CaptureError(
                        "no audio recording in progress".into(),
                    )),
                };
                let _ = reply.send(result);
            }
            CaptureControl::StartIq {
                meta_path,
                data_path,
                params,
                reply,
            } => {
                let result = if self.iq_writer.is_some() {
                    Err(RailError::CaptureError(
                        "IQ recording already in progress".into(),
                    ))
                } else {
                    match SigMfStreamWriter::create(&meta_path, &data_path, params) {
                        Ok(w) => {
                            self.iq_writer = Some(w);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                };
                let _ = reply.send(result);
            }
            CaptureControl::StopIq { reply } => {
                let result = match self.iq_writer.take() {
                    Some(w) => {
                        let meta_path = w.meta_path().to_path_buf();
                        let data_path = w.data_path().to_path_buf();
                        let sample_rate_hz = self.sample_rate_hz;
                        match w.finalize() {
                            Ok(samples) => Ok(IqStopInfo {
                                meta_path,
                                data_path,
                                samples,
                                sample_rate_hz,
                            }),
                            Err(e) => Err(e),
                        }
                    }
                    None => Err(RailError::CaptureError(
                        "no IQ recording in progress".into(),
                    )),
                };
                let _ = reply.send(result);
            }
        }
    }

    fn emit_signal_level(&mut self, rms_dbfs: f32) {
        if self.last_level_emit.elapsed() < MIN_LEVEL_EMIT_INTERVAL {
            return;
        }
        let current = if rms_dbfs.is_finite() { rms_dbfs } else { -120.0 };
        self.peak_dbfs = if self.peak_dbfs.is_finite() {
            (self.peak_dbfs - PEAK_DECAY_DB_PER_EMIT).max(current)
        } else {
            current
        };
        if let Err(e) = SignalLevel::new(current, self.peak_dbfs).emit(&self.app) {
            log::warn!("signal-level emit failed: {e}");
        }
        self.last_level_emit = Instant::now();
    }

    /// One-shot FFT + waterfall emit for a prefill window. Skips the
    /// rate limiter (`MIN_EMIT_INTERVAL`) that `emit_waterfall_frames`
    /// uses, and does not touch `fft_pending` / `last_emit` so a
    /// prefill burst doesn't starve the subsequent live-replay emits.
    /// Returns `false` if the channel is gone, matching the contract
    /// of `emit_waterfall_frames`.
    fn emit_prefill_frame(
        &mut self,
        samples: &[Complex<f32>],
        channel: &Channel<InvokeResponseBody>,
        canceler: Option<&IqCanceler>,
    ) -> bool {
        if samples.len() != FFT_SIZE {
            log::warn!(
                "prefill chunk has {} samples (expected {}); skipping",
                samples.len(),
                FFT_SIZE
            );
            return true;
        }
        match self.builder.process_shifted(samples) {
            Ok(spectrum) => {
                let bytes: &[u8] = cast_slice(spectrum);
                if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
                    log::warn!("waterfall channel send failed during prefill: {e}");
                    if let Some(c) = canceler {
                        c.cancel();
                    }
                    return false;
                }
            }
            Err(e) => log::warn!("prefill frame build failed: {e}"),
        }
        true
    }

    fn emit_waterfall_frames(
        &mut self,
        channel: &Channel<InvokeResponseBody>,
        canceler: Option<&IqCanceler>,
    ) -> bool {
        self.fft_pending.extend_from_slice(&self.shifted);

        while self.fft_pending.len() >= FFT_SIZE {
            let drop_frame = self.last_emit.elapsed() < MIN_EMIT_INTERVAL;
            let frame: Vec<Complex<f32>> = self.fft_pending.drain(..FFT_SIZE).collect();

            if drop_frame {
                continue;
            }

            match self.builder.process_shifted(&frame) {
                Ok(spectrum) => {
                    let bytes: &[u8] = cast_slice(spectrum);
                    if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
                        log::warn!("waterfall channel send failed: {e}; cancelling reader");
                        if let Some(c) = canceler {
                            c.cancel();
                        }
                        return false;
                    }
                    self.last_emit = Instant::now();
                }
                Err(e) => {
                    log::warn!("frame build failed: {e}");
                }
            }
        }
        true
    }

    fn emit_audio_chunks(
        &mut self,
        channel: &Channel<InvokeResponseBody>,
        canceler: Option<&IqCanceler>,
    ) -> bool {
        while self.audio_pending.len() >= AUDIO_CHUNK_SAMPLES {
            let tail: Vec<f32> = self.audio_pending.drain(..AUDIO_CHUNK_SAMPLES).collect();
            let bytes: &[u8] = cast_slice(&tail);
            if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
                log::warn!("audio channel send failed: {e}; cancelling reader");
                if let Some(c) = canceler {
                    c.cancel();
                }
                return false;
            }
        }
        true
    }
}

/// Register the AppState and all commands on a Tauri builder.
pub fn register<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .manage(AppState::default())
        .manage(BookmarksStore::default())
        .invoke_handler(tauri::generate_handler![
            ping,
            check_device,
            start_stream,
            stop_stream,
            set_gain,
            available_gains,
            retune,
            set_ppm,
            set_mode,
            set_bandwidth,
            set_squelch,
            list_bookmarks,
            add_bookmark,
            remove_bookmark,
            replace_bookmarks,
            start_audio_capture,
            stop_audio_capture,
            start_iq_capture,
            stop_iq_capture,
            finalize_capture,
            finalize_iq_capture,
            discard_capture,
            screenshot_suggestion,
            save_screenshot,
            open_replay,
            start_replay,
            pause_replay,
            resume_replay,
            seek_replay,
            stop_replay,
        ])
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
