//! Tauri command handlers (React → Rust) — session lifecycle and tuning.
//!
//! Streaming data flows back to the frontend through two per-session
//! `Channel<InvokeResponseBody>`s that the frontend passes to
//! [`start_stream`]: one for waterfall frames, one for f32 PCM audio.
//! See `docs/ARCHITECTURE.md` §3 and `docs/DSP.md` §4–5.
//!
//! Capture, replay, and the DSP worker live in sibling modules:
//! [`super::capture_cmd`], [`super::replay_cmd`], [`super::dsp_task`].

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::mpsc;

use crate::bookmarks::{Bookmark, BookmarksStore};
use crate::dsp::demod::{DemodChain, DemodMode, AUDIO_RATE_HZ};
use crate::dsp::input::DspInput;
use crate::error::RailError;
use crate::hardware::stream::{
    IqStream, DEFAULT_USB_BUF_LEN, DEFAULT_USB_BUF_NUM, IQ_CHANNEL_CAPACITY,
};
use crate::hardware::{self, DeviceInfo, RtlSdrDevice, RtlSdrTuner, Tuner};
use crate::ipc::control::{DspControl, DspControlHandle, RadioParams};
use crate::ipc::dsp_task::{spawn_dsp_task, DspTaskCfg, AUDIO_CHUNK_SAMPLES, FFT_SIZE};
use crate::ipc::events::DeviceStatus;
use crate::replay::{ReplayControl, ReplayInfo};

// Eagerly bring `DemodChain` into scope for the compiler check that the
// `dsp::demod` imports stay in sync with what the crate graph exposes.
// (No runtime use — `DemodChain` is consumed inside [`dsp_task`].)
#[allow(dead_code)]
type _DemodChainMarker = DemodChain;

/// Default RTL-SDR sample rate. Stable per `docs/HARDWARE.md` §4.
const DEFAULT_SAMPLE_RATE_HZ: u32 = 2_048_000;
/// Default channel bandwidth (Hz) a new session starts at — WBFM
/// broadcast. Must match `DemodConfig::default()` in `dsp::demod`.
const DEFAULT_BANDWIDTH_HZ: u32 = 200_000;
/// Fallback sample rates to probe if the requested one is rejected by
/// librtlsdr on a specific tuner/driver combo (`set_sample_rate -> -1`).
/// Ordered by preference.
const FALLBACK_SAMPLE_RATES_HZ: [u32; 5] = [2_048_000, 1_800_000, 1_400_000, 1_024_000, 900_000];

/// Mode names accepted over the wire. Kept in sync with
/// `src/store/radio.ts :: DemodMode`.
pub(crate) fn parse_mode(s: &str) -> Result<DemodMode, RailError> {
    match s {
        "FM" => Ok(DemodMode::Fm),
        "NFM" => Ok(DemodMode::Nfm),
        "AM" => Ok(DemodMode::Am),
        "USB" => Ok(DemodMode::Usb),
        "LSB" => Ok(DemodMode::Lsb),
        "CW" => Ok(DemodMode::Cw),
        other => Err(RailError::InvalidParameter(format!(
            "unknown mode: {other}"
        ))),
    }
}

/// One running streaming session. Held inside [`AppState`].
///
/// A session is either *live* (RTL-SDR reader + tuner hardware) or
/// *replay* (SigMF file reader). The DSP-facing fields (`dsp`,
/// `control`) are shared so the parameter and capture commands do not
/// care which source is running. Every runtime parameter lives behind
/// [`control`](Session::control) — the session keeps no shadow copy.
/// The [`source`](Session::source) enum only covers the bits that
/// differ between the two modes.
pub(crate) struct Session {
    /// JoinHandle for the DSP task (stops when the IQ sender drops).
    pub(crate) dsp: Option<tokio::task::JoinHandle<()>>,
    /// Sample rate of the IQ stream feeding the DSP task.
    pub(crate) sample_rate_hz: u32,
    /// The one control seam into the DSP worker: parameter and capture
    /// messages out, plus the parameter state-of-record the capture
    /// commands read. See [`crate::ipc::control`].
    pub(crate) control: DspControlHandle,
    /// Source-specific bits (live hardware vs replay file).
    pub(crate) source: SessionSource,
    /// Per-bin peak dBFS accumulator shared with the scanner task.
    /// The DSP task updates this every waterfall frame; the scanner resets
    /// it after settle and reads it at dwell end for burst-aware detection.
    pub(crate) max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
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
    tuner: Option<RtlSdrTuner>,
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
    /// Active scanner task, if any. Held separately from `session` to
    /// avoid deadlocks between the scanner and command handlers.
    pub(crate) scanner: Mutex<Option<crate::scanner::ScannerHandle>>,
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
    {
        let guard = state.session.lock().map_err(session_poisoned)?;
        if guard.is_some() {
            return Err(RailError::InvalidParameter("stream already running".into()));
        }
    } // drop guard before any await points

    let requested_sample_rate_hz = args.sample_rate_hz.unwrap_or(DEFAULT_SAMPLE_RATE_HZ);

    // On Windows with WinUSB, `rtlsdr_open` can succeed while the USB
    // endpoint is still settling — the first register write then returns
    // LIBUSB_ERROR_PIPE (-9).  Retry up to 3 times with a 100 ms gap;
    // the device is always ready within one retry in practice.
    // See `docs/HARDWARE.md` §6 ("rtlsdr_demod_write_reg failed with -9").
    const OPEN_RETRIES: usize = 3;
    let mut last_error = RailError::DeviceNotFound;
    let mut open_result: Option<(RtlSdrDevice, u32)> = None;

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
            open_result = Some((dev, rate));
            break;
        }
        // All rates failed — likely USB pipe error; drop `dev` and retry.
    }

    let (device, sample_rate) = open_result.ok_or(last_error)?;
    let offset = lo_offset_hz(sample_rate);
    // Park the LO `fs/4` below the user's target; the `−fs/4` digital
    // mixer in `apply_fs4_shift` brings the tuned carrier back to DC
    // with the hardware DC spike off-center (docs/DSP.md §1).
    let tuner = device.tuner();
    tuner.set_center_freq(args.frequency_hz.saturating_sub(offset))?;
    tuner.set_tuner_gain_mode(false)?;
    let gains = device.available_gains().unwrap_or_default();

    let actual_freq = tuner.center_freq().saturating_add(offset);

    let (iq_tx, iq_rx) = mpsc::channel::<DspInput>(IQ_CHANNEL_CAPACITY);
    // Seed the seam with the same defaults `DemodConfig::default()` uses,
    // so the worker and the state-of-record agree from the first sample.
    let (control, control_rx) = DspControlHandle::new(RadioParams {
        center_hz: actual_freq,
        mode: DemodMode::Fm,
        bandwidth_hz: DEFAULT_BANDWIDTH_HZ,
        squelch_dbfs: None,
        gain_tenths_db: None,
        ppm: 0,
    });

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

    let max_dbfs_per_bin = Arc::new(Mutex::new(vec![f32::NEG_INFINITY; FFT_SIZE]));

    let dsp_handle = spawn_dsp_task(DspTaskCfg {
        app: app.clone(),
        iq_rx,
        waterfall_channel,
        audio_channel,
        control_rx,
        canceler: Some(canceler),
        sample_rate_hz: sample_rate,
        initial_params: control.snapshot()?,
        max_dbfs_per_bin: max_dbfs_per_bin.clone(),
    });

    let mut guard = state.session.lock().map_err(session_poisoned)?;
    *guard = Some(Session {
        dsp: Some(dsp_handle),
        sample_rate_hz: sample_rate,
        control,
        source: SessionSource::Live(LiveBits {
            stream: Some(stream),
            tuner: Some(tuner),
            gains: gains.clone(),
        }),
        max_dbfs_per_bin,
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
    // Cancel any running scanner before tearing down the session it depends on.
    // The mutex guard is a temporary of this statement, so it is released
    // before the await below (`clippy::await_holding_lock` enforces that).
    let scanner = state.scanner.lock().ok().and_then(|mut g| g.take());
    if let Some(h) = scanner {
        h.cancel.store(true, Ordering::Relaxed);
        // Await it: the sweep holds an `RtlSdrTuner` over the device pointer,
        // so closing the device before the task exits is a use-after-close.
        let _ = h.handle.await;
    }

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
    // The hardware call stays here — the worker holds no tuner, and the
    // validation errors below have to reach the UI synchronously. Only
    // the state-of-record travels the seam.
    if args.auto {
        session.control.send(DspControl::SetGainTenthsDb(None))
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
        session
            .control
            .send(DspControl::SetGainTenthsDb(Some(tenths)))
    }
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
    session
        .control
        .send(DspControl::Retune { center_hz: freq })?;
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
    let mut guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_mut()
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

    tuner.set_freq_correction_ppm(args.ppm)?;
    session.control.send(DspControl::SetPpm(args.ppm))
}

/// Arguments for [`set_mode`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetModeArgs {
    pub mode: String,
}

#[tauri::command]
pub fn set_mode(args: SetModeArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    send_control(&state, DspControl::SetMode(parse_mode(&args.mode)?))
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
    send_control(&state, DspControl::SetBandwidthHz(args.bandwidth_hz as f32))
}

/// Arguments for [`set_squelch`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSquelchArgs {
    pub threshold_dbfs: Option<f32>,
}

#[tauri::command]
pub fn set_squelch(args: SetSquelchArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    send_control(
        &state,
        DspControl::SetSquelchDbfs(args.threshold_dbfs.filter(|v| v.is_finite())),
    )
}

/// Forward one message onto the session's control seam.
fn send_control(state: &State<'_, AppState>, msg: DspControl) -> Result<(), RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    session.control.send(msg)
}

/// Arguments for [`add_bookmark`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddBookmarkArgs {
    pub name: String,
    pub frequency_hz: u32,
    /// Demodulation mode at save time — forwarded as-is (no validation needed;
    /// backend stores whatever the frontend sends).
    pub mode: Option<String>,
    /// Filter bandwidth in Hz at save time.
    pub bandwidth_hz: Option<u32>,
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
    store.add(
        &app,
        args.name,
        args.frequency_hz,
        args.mode,
        args.bandwidth_hz,
    )
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

fn scanner_poisoned<T>(_: std::sync::PoisonError<T>) -> RailError {
    RailError::StreamError("scanner lock poisoned".into())
}

/// Start a wideband frequency sweep. Requires an active live stream.
///
/// The scanner tunes through `args.start_hz..=args.stop_hz` in steps of
/// `args.step_hz`, dwells `args.dwell_ms` at each step, and emits 8 bytes
/// per step on `scan_channel` (two little-endian f32: signal_avg_db and
/// noise_floor_db). SNR = signal_avg_db − noise_floor_db.
/// When a full sweep completes, emits the `scan-complete` JSON event.
/// When a step's SNR exceeds `args.squelch_snr_db`, emits `scan-stopped`.
/// See `docs/TIMELINE.md` Phase 9.
#[tauri::command]
pub async fn start_scan<R: Runtime>(
    app: AppHandle<R>,
    args: crate::scanner::StartScanArgs,
    scan_channel: Channel<InvokeResponseBody>,
    state: State<'_, AppState>,
) -> Result<crate::scanner::ScanStartReply, RailError> {
    if args.step_hz < 1_000 {
        return Err(RailError::InvalidParameter(
            "step_hz must be >= 1 000".into(),
        ));
    }
    if args.dwell_ms < 50 {
        return Err(RailError::InvalidParameter("dwell_ms must be >= 50".into()));
    }
    if args.start_hz >= args.stop_hz {
        return Err(RailError::InvalidParameter(
            "start_hz must be less than stop_hz".into(),
        ));
    }

    // Extract what the scanner task needs from the live session.
    let (tuner, lo_offset, max_dbfs_per_bin, sample_rate_hz, control) = {
        let guard = state.session.lock().map_err(session_poisoned)?;
        let session = guard
            .as_ref()
            .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
        let live = match &session.source {
            SessionSource::Live(l) => l,
            SessionSource::Replay(_) => {
                return Err(RailError::InvalidParameter(
                    "scanner is not available during replay".into(),
                ))
            }
        };
        let tuner = live
            .tuner
            .ok_or_else(|| RailError::InvalidParameter("tuner unavailable".into()))?;
        let lo_offset = lo_offset_hz(session.sample_rate_hz);
        let sample_rate_hz = session.sample_rate_hz;
        (
            tuner,
            lo_offset,
            session.max_dbfs_per_bin.clone(),
            sample_rate_hz,
            session.control.clone(),
        )
    };

    // Cancel any previous scan.
    {
        let prev = state.scanner.lock().map_err(scanner_poisoned)?.take();
        if let Some(h) = prev {
            h.cancel.store(true, Ordering::Relaxed);
        }
    }

    let frequencies_hz =
        crate::scanner::build_frequency_list(args.start_hz, args.stop_hz, args.step_hz);
    let reply = crate::scanner::ScanStartReply {
        frequencies_hz: frequencies_hz.clone(),
    };

    let handle = crate::scanner::spawn_scanner(
        app,
        tuner,
        lo_offset,
        frequencies_hz,
        args.dwell_ms,
        args.squelch_snr_db,
        max_dbfs_per_bin,
        scan_channel,
        sample_rate_hz,
        args.step_hz,
        control,
    );

    *state.scanner.lock().map_err(scanner_poisoned)? = Some(handle);

    Ok(reply)
}

/// Cancel an in-progress frequency sweep. Idempotent.
#[tauri::command]
pub async fn stop_scan(state: State<'_, AppState>) -> Result<(), RailError> {
    let handle = state.scanner.lock().map_err(scanner_poisoned)?.take();
    if let Some(h) = handle {
        h.cancel.store(true, Ordering::Relaxed);
        let _ = h.handle.await;
    }
    Ok(())
}

/// Register the AppState and all commands on a Tauri builder.
///
/// Commands from sibling modules are referenced via fully-qualified
/// paths because `#[tauri::command]` expands into a helper macro next
/// to the function, and `use` imports don't bring the macro into scope.
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
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
            start_scan,
            stop_scan,
        ])
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
