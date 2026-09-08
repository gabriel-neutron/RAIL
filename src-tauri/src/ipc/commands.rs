//! Tauri command handlers (React → Rust) — session lifecycle and tuning.
//!
//! Streaming data flows back to the frontend through two per-session
//! `Channel<InvokeResponseBody>`s that the frontend passes to
//! [`start_stream`](super::session::live::start_stream): one for
//! waterfall frames, one for f32 PCM audio.
//! See `docs/ARCHITECTURE.md` §3 and `docs/DSP.md` §4–5.
//!
//! Session assembly, capture, replay and the DSP worker live in sibling
//! modules: [`super::session`], [`super::capture_cmd`],
//! [`super::replay_cmd`], [`super::dsp_task`].

use std::sync::atomic::Ordering;

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};

use crate::bookmarks::{Bookmark, BookmarksStore};
use crate::dsp::demod::DemodMode;
use crate::error::RailError;
use crate::hardware::{self, DeviceInfo, Tuner};
use crate::ipc::control::DspControl;
use crate::ipc::session::live::lo_offset_hz;
use crate::ipc::session::{session_poisoned, AppState, SessionSource};
use crate::replay::ReplayControl;

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

/// Stop the streaming session and release the hardware. Idempotent.
///
/// Tears down live and replay sessions alike. No device-status event is
/// emitted — `stop_stream` is always an intentional frontend-initiated
/// teardown, so the caller already knows the stream ended (see the
/// `DeviceStatus::disconnected_with` path in the live reader's
/// disconnect closure for the genuine-disconnect case).
#[tauri::command]
pub async fn stop_stream(state: State<'_, AppState>) -> Result<(), RailError> {
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
            crate::ipc::session::live::start_stream,
            stop_stream,
            set_gain,
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
            crate::ipc::replay_cmd::replay_transport,
            start_scan,
            stop_scan,
        ])
}
