//! Tauri command handlers (React → Rust).
//!
//! Streaming data flows back to the frontend through two per-session
//! `Channel<InvokeResponseBody>`s that the frontend passes to
//! [`start_stream`]: one for waterfall frames, one for f32 PCM audio.
//! See `docs/ARCHITECTURE.md` §3 and `docs/DSP.md` §4–5.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use bytemuck::cast_slice;
use num_complex::Complex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::mpsc;

use crate::bookmarks::{Bookmark, BookmarksStore};
use crate::dsp::demod::{DemodChain, DemodControl, DemodMode, AUDIO_RATE_HZ};
use crate::dsp::waterfall::{apply_fs4_shift, iq_u8_to_complex, FrameBuilder};
use crate::error::RailError;
use crate::hardware::stream::{
    IqCanceler, IqStream, DEFAULT_USB_BUF_LEN, DEFAULT_USB_BUF_NUM, IQ_CHANNEL_CAPACITY,
};
use crate::hardware::{self, DeviceInfo, RtlSdrDevice, TunerHandle};
use crate::ipc::events::{DeviceStatus, SignalLevel};

/// FFT size (bins). Matches `docs/DSP.md` §2 default.
const FFT_SIZE: usize = 2048;

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
    /// Current sample rate; needed to compute the LO offset on `retune`.
    sample_rate_hz: u32,
    /// Outbound channel for runtime demod control (mode/bandwidth/squelch).
    control_tx: mpsc::UnboundedSender<DemodControl>,
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
///
/// The mutex guard is held across the whole body so two concurrent
/// `start_stream` invocations can't both pass the "slot is empty" check.
/// The body has no `.await` points, so holding the lock is cheap.
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
    // Start in auto gain; the UI can switch to manual via `set_gain`.
    device.set_tuner_gain_mode(false)?;
    let gains = device.available_gains().unwrap_or_default();

    let tuner = device.tuner_handle();
    let actual_freq = device.center_freq().saturating_sub(offset);

    let (iq_tx, iq_rx) = mpsc::channel::<Vec<u8>>(IQ_CHANNEL_CAPACITY);
    let (control_tx, control_rx) = mpsc::unbounded_channel::<DemodControl>();

    // Fires from the reader thread if the dongle is unplugged mid-stream.
    // Emits a `device-status` event so the UI can switch back to the
    // "missing device" view without the process crashing in `rtlsdr_close`.
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
        canceler,
        sample_rate as f32,
    );

    *guard = Some(Session {
        stream: Some(stream),
        dsp: Some(dsp_handle),
        tuner: Some(tuner),
        gains: gains.clone(),
        sample_rate_hz: sample_rate,
        control_tx,
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

/// Arguments for [`retune`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetuneArgs {
    pub frequency_hz: u32,
}

/// Reply for [`retune`]. Reports the frequency librtlsdr actually snapped
/// to — may differ from the request (`docs/HARDWARE.md` §4).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetuneReply {
    pub frequency_hz: u32,
}

/// Retune the live session to a new center frequency without tearing the
/// IQ stream down. See `docs/DSP.md` §1 for the LO/DC-offset context.
///
/// Safe against the running `read_async` loop: librtlsdr serializes tuner
/// commands internally (see [`TunerHandle`] doc).
#[tauri::command]
pub fn retune(args: RetuneArgs, state: State<'_, AppState>) -> Result<RetuneReply, RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    let tuner = session
        .tuner
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("tuner unavailable".into()))?;

    let offset = lo_offset_hz(session.sample_rate_hz);
    tuner.set_center_freq(args.frequency_hz.saturating_add(offset))?;
    Ok(RetuneReply {
        frequency_hz: tuner.center_freq().saturating_sub(offset),
    })
}

/// Arguments for [`set_ppm`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPpmArgs {
    pub ppm: i32,
}

/// Apply a PPM crystal correction to the live session.
/// See `docs/HARDWARE.md` §3.
#[tauri::command]
pub fn set_ppm(args: SetPpmArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    let session = guard
        .as_ref()
        .ok_or_else(|| RailError::InvalidParameter("stream not running".into()))?;
    let tuner = session
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

/// Switch the active demodulator. Only FM/AM are wired for Phase 3;
/// USB/LSB/CW return [`RailError::InvalidParameter`].
#[tauri::command]
pub fn set_mode(args: SetModeArgs, state: State<'_, AppState>) -> Result<(), RailError> {
    let mode = parse_mode(&args.mode)?;
    send_control(&state, DemodControl::SetMode(mode))
}

/// Arguments for [`set_bandwidth`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBandwidthArgs {
    pub bandwidth_hz: u32,
}

/// Set the channel-filter bandwidth in Hz.
///
/// Also implicitly selects WBFM vs NBFM when mode == FM — bandwidths
/// ≥ 100 kHz activate de-emphasis and 75 kHz max deviation.
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
    send_control(
        &state,
        DemodControl::SetBandwidthHz(args.bandwidth_hz as f32),
    )
}

/// Arguments for [`set_squelch`].
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSquelchArgs {
    /// `None` or non-finite disables squelch. Otherwise a dBFS
    /// threshold — audio is gated to zero below this level.
    pub threshold_dbfs: Option<f32>,
}

/// Set the channel-power squelch threshold in dBFS. `None` or `NaN`
/// disables the gate.
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

/// Return the full bookmark list, sorted by creation time.
#[tauri::command]
pub fn list_bookmarks<R: Runtime>(
    app: AppHandle<R>,
    store: State<'_, BookmarksStore>,
) -> Result<Vec<Bookmark>, RailError> {
    store.list(&app)
}

/// Persist a new bookmark and return it (with the generated id).
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

/// Remove a bookmark by id (idempotent — missing id is not an error).
#[tauri::command]
pub fn remove_bookmark<R: Runtime>(
    app: AppHandle<R>,
    args: RemoveBookmarkArgs,
    store: State<'_, BookmarksStore>,
) -> Result<(), RailError> {
    store.remove(&app, &args.id)
}

/// Arguments for [`replace_bookmarks`]. Accepts any list shape the
/// frontend parsed from a user-supplied JSON file.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceBookmarksArgs {
    pub bookmarks: Vec<Bookmark>,
}

/// Overwrite the on-disk bookmark list with the supplied one. Used by
/// the "Load" menu entry when the user picks a JSON file.
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

/// Spawn the shared DSP worker: converts IQ bytes once, applies the
/// `fs/4` shift once, then fans out to the waterfall FFT and to the
/// demod chain. Control messages from `set_mode`/`set_bandwidth`/
/// `set_squelch` are drained at the top of every iteration.
fn spawn_dsp_task<R: Runtime>(
    app: AppHandle<R>,
    iq_rx: mpsc::Receiver<Vec<u8>>,
    waterfall_channel: Channel<InvokeResponseBody>,
    audio_channel: Channel<InvokeResponseBody>,
    control_rx: mpsc::UnboundedReceiver<DemodControl>,
    canceler: IqCanceler,
    sample_rate_hz: f32,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        // FFT + demod work is CPU-bound, so living on the blocking
        // pool avoids starving other async tasks.
        let mut ctx = DspTaskCtx::<R>::new(app, sample_rate_hz);
        ctx.run(
            iq_rx,
            waterfall_channel,
            audio_channel,
            control_rx,
            canceler,
        );
    })
}

/// Internal holder for the DSP worker's mutable state. Keeps the
/// `spawn_dsp_task` closure trivial and lets us unit-test helpers
/// independently if we ever need to.
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
    /// Backend peak-hold for the signal meter. Decays each time
    /// we emit a `signal-level` event (see `PEAK_DECAY_DB_PER_EMIT`).
    peak_dbfs: f32,
    last_level_emit: Instant,
}

impl<R: Runtime> DspTaskCtx<R> {
    fn new(app: AppHandle<R>, sample_rate_hz: f32) -> Self {
        Self {
            app,
            builder: FrameBuilder::new(FFT_SIZE),
            chain: DemodChain::new(sample_rate_hz),
            shifted: Vec::with_capacity(DEFAULT_USB_BUF_LEN as usize / 2),
            fft_pending: Vec::with_capacity(FFT_SIZE * 2),
            audio_pending: Vec::with_capacity(AUDIO_CHUNK_SAMPLES * 2),
            phase_idx: 0,
            last_emit: Instant::now() - MIN_EMIT_INTERVAL,
            peak_dbfs: f32::NEG_INFINITY,
            last_level_emit: Instant::now() - MIN_LEVEL_EMIT_INTERVAL,
        }
    }

    fn run(
        &mut self,
        mut iq_rx: mpsc::Receiver<Vec<u8>>,
        waterfall_channel: Channel<InvokeResponseBody>,
        audio_channel: Channel<InvokeResponseBody>,
        mut control_rx: mpsc::UnboundedReceiver<DemodControl>,
        canceler: IqCanceler,
    ) {
        while let Some(chunk) = iq_rx.blocking_recv() {
            // Apply any pending control changes atomically before
            // consuming the next chunk.
            while let Ok(msg) = control_rx.try_recv() {
                self.chain.apply(msg);
            }

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

            // Fan out branch 1: waterfall FFT (rate-limited).
            if !self.emit_waterfall_frames(&waterfall_channel, &canceler) {
                return;
            }

            // Fan out branch 2: demod → audio (always ships whatever
            // the chain produced this iteration).
            let rms_dbfs = self.chain.process(&self.shifted, &mut self.audio_pending);
            if !self.emit_audio_chunks(&audio_channel, &canceler) {
                return;
            }

            // Fan out branch 3: signal-level JSON event (rate-limited
            // to ~25 Hz, with a simple decaying peak-hold).
            self.emit_signal_level(rms_dbfs);
        }

        log::debug!("dsp task exiting: iq sender dropped");
    }

    /// Push a `signal-level` event if enough time has elapsed, folding
    /// the new `rms_dbfs` reading into a decaying peak-hold.
    fn emit_signal_level(&mut self, rms_dbfs: f32) {
        if self.last_level_emit.elapsed() < MIN_LEVEL_EMIT_INTERVAL {
            return;
        }
        // Non-finite (NEG_INFINITY on silence) collapses to the floor
        // so the frontend doesn't have to special-case it.
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

    /// Append the shifted IQ to the FFT pending buffer and emit as
    /// many FFT frames as we have (capped to `MIN_EMIT_INTERVAL`).
    /// Returns `false` if the waterfall channel is closed.
    fn emit_waterfall_frames(
        &mut self,
        channel: &Channel<InvokeResponseBody>,
        canceler: &IqCanceler,
    ) -> bool {
        self.fft_pending.extend_from_slice(&self.shifted);

        while self.fft_pending.len() >= FFT_SIZE {
            // Always drain the frame even when the emission is
            // rate-limited, so the pending buffer doesn't grow
            // unbounded (docs/DSP.md §3).
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
                        canceler.cancel();
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

    /// Ship accumulated audio in `AUDIO_CHUNK_SAMPLES`-sized bursts.
    /// Returns `false` if the audio channel is closed.
    fn emit_audio_chunks(
        &mut self,
        channel: &Channel<InvokeResponseBody>,
        canceler: &IqCanceler,
    ) -> bool {
        while self.audio_pending.len() >= AUDIO_CHUNK_SAMPLES {
            let tail: Vec<f32> = self.audio_pending.drain(..AUDIO_CHUNK_SAMPLES).collect();
            let bytes: &[u8] = cast_slice(&tail);
            if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
                log::warn!("audio channel send failed: {e}; cancelling reader");
                canceler.cancel();
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
        ])
}
