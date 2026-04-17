//! Tauri command handlers (React → Rust).
//!
//! Streaming data flows back to the frontend through a per-session
//! `Channel<InvokeResponseBody>` that the frontend passes to
//! [`start_stream`]. See `docs/ARCHITECTURE.md` §3.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use bytemuck::cast_slice;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::mpsc;

use crate::dsp::waterfall::FrameBuilder;
use crate::error::RailError;
use crate::hardware::stream::{
    IqCanceler, IqStream, DEFAULT_USB_BUF_LEN, DEFAULT_USB_BUF_NUM, IQ_CHANNEL_CAPACITY,
};
use crate::hardware::{self, DeviceInfo, RtlSdrDevice, TunerHandle};
use crate::ipc::events::DeviceStatus;

/// FFT size (bins). Matches `docs/DSP.md` §2 default.
const FFT_SIZE: usize = 2048;

/// Default RTL-SDR sample rate. Stable per `docs/HARDWARE.md` §4.
const DEFAULT_SAMPLE_RATE_HZ: u32 = 2_048_000;

/// Minimum interval between frames emitted to the frontend (~25 fps cap,
/// `docs/DSP.md` §3).
const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(40);

/// One running streaming session. Held inside [`AppState`].
struct Session {
    /// RAII for the reader thread. Option so we can take it out in `stop`.
    stream: Option<IqStream>,
    /// JoinHandle for the DSP task (stops when the IQ sender drops).
    dsp: Option<tokio::task::JoinHandle<()>>,
    /// Thread-safe tuning surface. `None` after the session is torn down
    /// so that late `set_gain` calls return an error instead of racing
    /// with device close.
    tuner: Option<TunerHandle>,
    /// Discrete gain steps the hardware supports (tenths of dB).
    gains: Vec<i32>,
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

/// Parameters for [`start_stream`]. Kept as a struct so adding fields in
/// Phase 2 doesn't change the command signature.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartStreamArgs {
    pub frequency_hz: u32,
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
}

/// Reply for [`start_stream`]. Tells the frontend what FFT size to expect
/// on the channel and which gain steps are available.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartStreamReply {
    pub fft_size: usize,
    pub sample_rate_hz: u32,
    pub frequency_hz: u32,
    pub available_gains_tenths_db: Vec<i32>,
}

/// Open the first RTL-SDR, configure it, and start the IQ → FFT → channel
/// pipeline. Returns metadata the UI needs to render the waterfall.
///
/// The mutex guard is held across the whole body so two concurrent
/// `start_stream` invocations can't both pass the "slot is empty" check.
/// The body has no `.await` points, so holding the lock is cheap.
#[tauri::command]
pub async fn start_stream<R: Runtime>(
    app: AppHandle<R>,
    args: StartStreamArgs,
    channel: Channel<InvokeResponseBody>,
    state: State<'_, AppState>,
) -> Result<StartStreamReply, RailError> {
    let mut guard = state.session.lock().map_err(session_poisoned)?;
    if guard.is_some() {
        return Err(RailError::InvalidParameter(
            "stream already running".into(),
        ));
    }

    let sample_rate = args.sample_rate_hz.unwrap_or(DEFAULT_SAMPLE_RATE_HZ);

    let device = RtlSdrDevice::open(0)?;
    device.set_sample_rate(sample_rate)?;
    device.set_center_freq(args.frequency_hz)?;
    // Start in auto gain; the UI can switch to manual via `set_gain`.
    device.set_tuner_gain_mode(false)?;
    let gains = device.available_gains().unwrap_or_default();

    let tuner = device.tuner_handle();
    let actual_freq = device.center_freq();

    let (iq_tx, iq_rx) = mpsc::channel::<Vec<u8>>(IQ_CHANNEL_CAPACITY);

    let stream = IqStream::start(device, iq_tx, DEFAULT_USB_BUF_NUM, DEFAULT_USB_BUF_LEN)?;
    let canceler = stream.canceler();

    let dsp_handle = spawn_dsp_task(iq_rx, channel, canceler);

    *guard = Some(Session {
        stream: Some(stream),
        dsp: Some(dsp_handle),
        tuner: Some(tuner),
        gains: gains.clone(),
    });
    drop(guard);

    let _ = DeviceStatus::connected().emit(&app);

    Ok(StartStreamReply {
        fft_size: FFT_SIZE,
        sample_rate_hz: sample_rate,
        frequency_hz: actual_freq,
        available_gains_tenths_db: gains,
    })
}

/// Stop the streaming session and release the hardware. Idempotent:
/// calling it on an already-stopped session is a no-op.
#[tauri::command]
pub async fn stop_stream<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), RailError> {
    let session = {
        let mut guard = state.session.lock().map_err(session_poisoned)?;
        guard.take()
    };
    let Some(mut session) = session else {
        return Ok(());
    };

    session.tuner.take();

    // Always wait on the DSP task even if the reader stop failed, so we
    // don't leak a detached tokio task holding the channel.
    let stream_result = session
        .stream
        .take()
        .map(|s| s.stop())
        .unwrap_or(Ok(()));
    if let Some(dsp) = session.dsp.take() {
        let _ = dsp.await;
    }

    let _ = DeviceStatus::disconnected_with("stream stopped").emit(&app);
    stream_result
}

/// Arguments for [`set_gain`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetGainArgs {
    pub auto: bool,
    #[serde(default)]
    pub tenths_db: Option<i32>,
}

/// Switch gain mode between AGC and manual. When `auto == false`,
/// `tenths_db` is required.
#[tauri::command]
pub fn set_gain(args: SetGainArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    let tuner = session
        .tuner
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("tuner unavailable".into()))?;

    tuner.set_tuner_gain_mode(!args.auto)?;
    if !args.auto {
        let tenths = args
            .tenths_db
            .ok_or_else(|| RailError::InvalidParameter("manual gain requires tenthsDb".into()))?;
        if !session.gains.is_empty() && !session.gains.contains(&tenths) {
            return Err(RailError::InvalidParameter(format!(
                "gain {tenths} not in supported set"
            )));
        }
        tuner.set_tuner_gain_tenths(tenths)?;
    }
    Ok(())
}

/// Report the gain steps supported by the current session's tuner.
/// Returns empty if no session is running.
#[tauri::command]
pub fn available_gains(state: State<'_, AppState>) -> Result<Vec<i32>, RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    Ok(guard.as_ref().map(|s| s.gains.clone()).unwrap_or_default())
}

fn session_poisoned<T>(_: std::sync::PoisonError<T>) -> RailError {
    RailError::StreamError("session lock poisoned".into())
}

fn spawn_dsp_task(
    mut iq_rx: mpsc::Receiver<Vec<u8>>,
    channel: Channel<InvokeResponseBody>,
    canceler: IqCanceler,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        // FFT + colormap work is CPU-bound, so living on the blocking
        // pool avoids starving other async tasks. `blocking_recv`
        // cooperates with a std thread.
        let mut builder = FrameBuilder::new(FFT_SIZE);
        let frame_bytes = builder.bytes_per_frame();
        let mut pending: Vec<u8> = Vec::with_capacity(frame_bytes * 2);
        let mut frame_scratch: Vec<u8> = vec![0u8; frame_bytes];
        let mut last_emit = Instant::now() - MIN_EMIT_INTERVAL;

        while let Some(mut chunk) = iq_rx.blocking_recv() {
            if pending.is_empty() {
                pending = std::mem::take(&mut chunk);
            } else {
                pending.append(&mut chunk);
            }

            while pending.len() >= frame_bytes {
                frame_scratch.copy_from_slice(&pending[..frame_bytes]);
                pending.drain(..frame_bytes);

                // 25 fps emission cap — still consume the frame to keep
                // the IQ pipeline draining (DSP.md §3).
                if last_emit.elapsed() < MIN_EMIT_INTERVAL {
                    continue;
                }

                match builder.build(&frame_scratch) {
                    Ok(spectrum) => {
                        let bytes: &[u8] = cast_slice(spectrum);
                        if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
                            // Frontend dropped the channel without calling
                            // stop_stream; tell the reader to shut down so
                            // we don't leak the USB transfer thread.
                            log::warn!("waterfall channel send failed: {e}; cancelling reader");
                            canceler.cancel();
                            return;
                        }
                        last_emit = Instant::now();
                    }
                    Err(e) => {
                        log::warn!("frame build failed: {e}");
                    }
                }
            }
        }

        log::debug!("dsp task exiting: iq sender dropped");
    })
}

/// Register the AppState and all Phase 1 commands on a Tauri builder.
pub fn register<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.manage(AppState::default()).invoke_handler(
        tauri::generate_handler![
            ping,
            check_device,
            start_stream,
            stop_stream,
            set_gain,
            available_gains,
        ],
    )
}
