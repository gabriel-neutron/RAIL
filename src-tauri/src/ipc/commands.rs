//! Tauri command handlers (React → Rust) — session lifecycle and tuning.
//!
//! Streaming data flows back to the frontend through two per-session
//! `Channel<InvokeResponseBody>`s that the frontend passes to
//! [`start_stream`]: one for waterfall frames, one for f32 PCM audio.
//! See `docs/ARCHITECTURE.md` §3 and `docs/DSP.md` §4–5.
//!
//! Capture, replay, and the DSP worker live in sibling modules:
//! [`super::capture_cmd`], [`super::replay_cmd`], [`super::dsp_task`].

use std::sync::Mutex;

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::mpsc;

use crate::bookmarks::{Bookmark, BookmarksStore};
use crate::dsp::demod::{DemodChain, DemodControl, DemodMode, AUDIO_RATE_HZ};
use crate::dsp::input::DspInput;
use crate::error::RailError;
use crate::hardware::stream::{
    IqStream, DEFAULT_USB_BUF_LEN, DEFAULT_USB_BUF_NUM, IQ_CHANNEL_CAPACITY,
};
use crate::hardware::{self, DeviceInfo, RtlSdrDevice, TunerHandle};
use crate::ipc::capture_cmd::CaptureControl;
use crate::ipc::dsp_task::{spawn_dsp_task, AUDIO_CHUNK_SAMPLES, FFT_SIZE};
use crate::ipc::events::DeviceStatus;
use crate::replay::{ReplayControl, ReplayInfo};

// Eagerly bring `DemodChain` into scope for the compiler check that the
// `dsp::demod` imports stay in sync with what the crate graph exposes.
// (No runtime use — `DemodChain` is consumed inside [`dsp_task`].)
#[allow(dead_code)]
type _DemodChainMarker = DemodChain;

/// Default RTL-SDR sample rate. Stable per `docs/HARDWARE.md` §4.
const DEFAULT_SAMPLE_RATE_HZ: u32 = 2_048_000;
/// Fallback sample rates to probe if the requested one is rejected by
/// librtlsdr on a specific tuner/driver combo (`set_sample_rate -> -1`).
/// Ordered by preference.
const FALLBACK_SAMPLE_RATES_HZ: [u32; 5] = [2_048_000, 1_800_000, 1_400_000, 1_024_000, 900_000];

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
pub(crate) struct Session {
    /// JoinHandle for the DSP task (stops when the IQ sender drops).
    pub(crate) dsp: Option<tokio::task::JoinHandle<()>>,
    /// Sample rate of the IQ stream feeding the DSP task.
    pub(crate) sample_rate_hz: u32,
    /// Outbound channel for runtime demod control (mode/bandwidth/squelch).
    pub(crate) control_tx: mpsc::UnboundedSender<DemodControl>,
    /// Outbound channel for capture-related requests (audio / IQ).
    pub(crate) capture_tx: mpsc::UnboundedSender<CaptureControl>,
    /// Most recent centre frequency, kept in sync with `retune` so the
    /// capture metadata and suggested filenames always match what the
    /// user sees.
    pub(crate) frequency_hz: u32,
    /// Most recent demod mode (`FM` / `AM`), updated on `set_mode`.
    pub(crate) mode: String,
    /// Most recent channel bandwidth (Hz), updated on `set_bandwidth`.
    pub(crate) bandwidth_hz: u32,
    /// Most recent manual gain (tenths of dB). `None` while in AGC.
    pub(crate) gain_tenths_db: Option<i32>,
    /// Source-specific bits (live hardware vs replay file).
    pub(crate) source: SessionSource,
}

/// Source-specific state for a [`Session`].
pub(crate) enum SessionSource {
    Live(LiveBits),
    Replay(ReplayBits),
}

pub(crate) struct LiveBits {
    /// RAII for the reader thread. Option so we can take it out in `stop`.
    stream: Option<IqStream>,
    /// Thread-safe tuning surface. `None` after the session is torn down
    /// so that late `set_gain` calls return an error instead of racing
    /// with device close.
    tuner: Option<TunerHandle>,
    /// Discrete gain steps the hardware supports (tenths of dB).
    gains: Vec<i32>,
}

pub(crate) struct ReplayBits {
    /// JoinHandle for the replay reader task.
    pub(crate) reader: Option<tokio::task::JoinHandle<()>>,
    /// Transport control channel (play/pause/seek/stop).
    pub(crate) control_tx: mpsc::UnboundedSender<ReplayControl>,
    /// Cached file metadata — handed back to the frontend on open /
    /// used to clamp seek positions without re-reading the file.
    pub(crate) info: ReplayInfo,
}

/// LO offset used to push the RTL-SDR DC spike off the center bin.
/// See `docs/DSP.md` §1 and the `fs/4` mixer in
/// [`crate::dsp::waterfall::apply_fs4_shift`].
fn lo_offset_hz(sample_rate_hz: u32) -> u32 {
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

/// Global, single-session state.
#[derive(Default)]
pub struct AppState {
    pub(crate) session: Mutex<Option<Session>>,
}

pub(crate) fn session_poisoned<T>(_: std::sync::PoisonError<T>) -> RailError {
    RailError::StreamError("session lock poisoned".into())
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
        return Err(RailError::InvalidParameter("stream already running".into()));
    }

    let requested_sample_rate_hz = args.sample_rate_hz.unwrap_or(DEFAULT_SAMPLE_RATE_HZ);

    let device = RtlSdrDevice::open(0)?;
    let mut applied_sample_rate_hz = None;
    let mut sample_rate_errors: Vec<String> = Vec::new();
    for candidate_hz in sample_rate_candidates(requested_sample_rate_hz) {
        match device.set_sample_rate(candidate_hz) {
            Ok(()) => {
                applied_sample_rate_hz = Some(candidate_hz);
                break;
            }
            Err(e) => sample_rate_errors.push(format!("{candidate_hz}: {e}")),
        }
    }
    let sample_rate = applied_sample_rate_hz.ok_or_else(|| {
        RailError::StreamError(format!(
            "failed to set sample rate (requested {requested_sample_rate_hz}): {}",
            sample_rate_errors.join(" | ")
        ))
    })?;
    if sample_rate != requested_sample_rate_hz {
        log::warn!(
            "requested sample rate {} rejected; using fallback {}",
            requested_sample_rate_hz,
            sample_rate
        );
    }
    let offset = lo_offset_hz(sample_rate);
    // Park the LO `fs/4` below the user's target; the `−fs/4` digital
    // mixer in `apply_fs4_shift` brings the tuned carrier back to DC
    // with the hardware DC spike off-center (docs/DSP.md §1).
    device.set_center_freq(args.frequency_hz.saturating_sub(offset))?;
    device.set_tuner_gain_mode(false)?;
    let gains = device.available_gains().unwrap_or_default();

    let tuner = device.tuner_handle();
    let actual_freq = device.center_freq().saturating_add(offset);

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
    tuner.set_center_freq(args.frequency_hz.saturating_sub(offset))?;
    let freq = tuner.center_freq().saturating_add(offset);
    session.frequency_hz = freq;
    Ok(RetuneReply { frequency_hz: freq })
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
pub fn set_bandwidth(args: SetBandwidthArgs, state: State<'_, AppState>) -> Result<(), RailError> {
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
pub fn set_squelch(args: SetSquelchArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    let db = args
        .threshold_dbfs
        .filter(|v| v.is_finite())
        .unwrap_or(f32::NEG_INFINITY);
    send_control(&state, DemodControl::SetSquelchDbfs(db))
}

fn send_control(state: &State<'_, AppState>, msg: DemodControl) -> Result<(), RailError> {
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

/// Register the AppState and all commands on a Tauri builder.
///
/// Commands from sibling modules are referenced via fully-qualified
/// paths because `#[tauri::command]` expands into a helper macro next
/// to the function, and `use` imports don't bring the macro into scope.
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
            crate::ipc::capture_cmd::start_audio_capture,
            crate::ipc::capture_cmd::stop_audio_capture,
            crate::ipc::capture_cmd::start_iq_capture,
            crate::ipc::capture_cmd::stop_iq_capture,
            crate::ipc::capture_cmd::finalize_capture,
            crate::ipc::capture_cmd::finalize_iq_capture,
            crate::ipc::capture_cmd::discard_capture,
            crate::ipc::capture_cmd::screenshot_suggestion,
            crate::ipc::capture_cmd::save_screenshot,
            crate::ipc::replay_cmd::open_replay,
            crate::ipc::replay_cmd::start_replay,
            crate::ipc::replay_cmd::pause_replay,
            crate::ipc::replay_cmd::resume_replay,
            crate::ipc::replay_cmd::seek_replay,
            crate::ipc::replay_cmd::stop_replay,
        ])
}

#[cfg(test)]
mod tests {
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
