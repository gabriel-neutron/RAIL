//! Replay transport commands.
//!
//! Opens a SigMF `.sigmf-data` file and drives the same DSP worker
//! that the live stream uses, via
//! [`crate::replay::spawn_replay_reader`] instead of the RTL-SDR.
//! See `docs/ARCHITECTURE.md` §3 and REVIEW_V1.md §5.3.

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::mpsc;

use crate::dsp::demod::{DemodControl, AUDIO_RATE_HZ};
use crate::dsp::input::DspInput;
use crate::error::RailError;
use crate::hardware::stream::IQ_CHANNEL_CAPACITY;
use crate::ipc::capture_cmd::CaptureControl;
use crate::ipc::commands::{
    session_poisoned, stop_stream, AppState, ReplayBits, Session, SessionSource,
};
use crate::ipc::dsp_task::{spawn_dsp_task, AUDIO_CHUNK_SAMPLES, FFT_SIZE};
use crate::ipc::events::DeviceStatus;
use crate::replay::{spawn_replay_reader, ReplayControl, ReplayInfo};

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
/// instead of the RTL-SDR. Mirrors `start_stream`'s shape: the caller
/// hands in two binary channels (waterfall, audio).
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
        SessionSource::Live(_) => Err(RailError::InvalidParameter("no replay in progress".into())),
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
pub fn seek_replay(args: SeekReplayArgs, state: State<'_, AppState>) -> Result<(), RailError> {
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
                return Err(RailError::InvalidParameter("no replay in progress".into()))
            }
        }
    };
    tx.send(ReplayControl::Seek { sample_idx })
        .map_err(|e| RailError::StreamError(format!("replay control channel closed: {e}")))
}

/// Tear down a replay session. Idempotent; behaves like `stop_stream`
/// for a live session.
#[tauri::command]
pub async fn stop_replay<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), RailError> {
    stop_stream(app, state).await
}
