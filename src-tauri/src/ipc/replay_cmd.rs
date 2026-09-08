//! Replay transport commands.
//!
//! Opens a SigMF `.sigmf-data` file and drives the same DSP worker
//! that the live stream uses, via
//! [`crate::replay::spawn_replay_reader`] instead of the RTL-SDR.
//! See `docs/ARCHITECTURE.md` §3.

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::mpsc;

use crate::dsp::demod::AUDIO_RATE_HZ;
use crate::error::RailError;
use crate::ipc::dsp_task::{AUDIO_CHUNK_SAMPLES, FFT_SIZE};
use crate::ipc::session::{
    ensure_idle, session_poisoned, start_session, AppState, ReplayBits, ScanAccumulator,
    SessionPlan, SessionSource,
};
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
    /// `demod_mode` and `filter_bandwidth_hz` report the *resolved*
    /// parameters ([`ReplayInfo::radio_params`]), not the raw sidecar
    /// fields. A file that names neither leaves those as the empty /
    /// zero "file did not say" sentinels, and the frontend must be told
    /// what the chain is actually running — the same defaults the
    /// session was seeded with.
    fn from_info(info: &ReplayInfo) -> Self {
        let params = info.radio_params();
        Self {
            data_path: info.data_path.to_string_lossy().into_owned(),
            meta_path: info.meta_path.to_string_lossy().into_owned(),
            sample_rate_hz: info.sample_rate_hz,
            center_frequency_hz: info.center_frequency_hz,
            demod_mode: params.mode_str().to_string(),
            filter_bandwidth_hz: params.bandwidth_hz,
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

/// The session plan a capture replays through.
///
/// The SigMF sidecar is the only source of truth for a replay session's
/// frequency, mode and bandwidth; replay cannot be scanned, so it takes
/// no per-bin accumulator. Extracted from [`start_replay`] so the
/// sidecar-to-chain wiring is testable without a Tauri runtime.
fn replay_plan(info: &ReplayInfo) -> SessionPlan {
    SessionPlan {
        sample_rate_hz: info.sample_rate_hz,
        params: info.radio_params(),
        scan_accumulator: ScanAccumulator::Empty,
    }
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
    ensure_idle(&state, "stop the current stream before opening a file")?;

    let info = crate::replay::load_info(std::path::Path::new(&args.data_path))?;
    let plan = replay_plan(&info);
    let center_hz = plan.params.center_hz;
    let (replay_ctl_tx, replay_ctl_rx) = mpsc::unbounded_channel::<ReplayControl>();

    let reader_info = info.clone();
    let reader_app = app.clone();
    start_session(
        &app,
        &state,
        plan,
        waterfall_channel,
        audio_channel,
        move |iq_tx| {
            let reader = spawn_replay_reader(reader_app, reader_info.clone(), iq_tx, replay_ctl_rx);
            // No hardware reader to cancel — the DSP task exits cleanly
            // when the replay reader drops its `iq_tx`, and
            // `ReplayControl::Stop` breaks out of the pacing loop.
            Ok((
                SessionSource::Replay(ReplayBits {
                    reader: Some(reader),
                    control_tx: replay_ctl_tx,
                    info: reader_info,
                }),
                None,
            ))
        },
    )?;

    Ok(StartReplayReply {
        fft_size: FFT_SIZE,
        sample_rate_hz: info.sample_rate_hz,
        frequency_hz: center_hz,
        audio_sample_rate_hz: AUDIO_RATE_HZ as u32,
        audio_chunk_samples: AUDIO_CHUNK_SAMPLES,
        info: ReplayInfoReply::from_info(&info),
    })
}

/// One transport verb for a running replay session.
///
/// Replaces the four pass-through commands (`pause`, `resume`, `seek`,
/// `stop`); teardown is `stop_stream`, which is not a transport verb —
/// it releases hardware and joins the worker tasks.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReplayTransportArgs {
    Play,
    Pause,
    #[serde(rename_all = "camelCase")]
    Seek {
        position_ms: u64,
    },
}

/// Send one transport verb to the running replay session.
///
/// Errors when no session is installed or when the running session is
/// live rather than replay.
#[tauri::command]
pub fn replay_transport(
    args: ReplayTransportArgs,
    state: State<'_, AppState>,
) -> Result<(), RailError> {
    let (tx, control) = {
        let guard = state.session.lock().map_err(session_poisoned)?;
        let session = guard
            .as_ref()
            .ok_or_else(|| RailError::InvalidParameter("no session running".into()))?;
        let replay = match &session.source {
            SessionSource::Replay(r) => r,
            SessionSource::Live(_) => {
                return Err(RailError::InvalidParameter("no replay in progress".into()))
            }
        };
        let control = match args {
            ReplayTransportArgs::Play => ReplayControl::Play,
            ReplayTransportArgs::Pause => ReplayControl::Pause,
            // Clamp against the cached total_samples inside the session
            // so a stale slider value can't confuse the reader.
            ReplayTransportArgs::Seek { position_ms } => ReplayControl::Seek {
                sample_idx: crate::replay::ms_to_sample_idx(
                    position_ms,
                    replay.info.sample_rate_hz,
                    replay.info.total_samples,
                ),
            },
        };
        (replay.control_tx.clone(), control)
    };
    tx.send(control)
        .map_err(|e| RailError::StreamError(format!("replay control channel closed: {e}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::{replay_plan, ReplayInfoReply, ReplayTransportArgs};
    use crate::dsp::demod::DemodMode;
    use crate::ipc::control::{DspParamState, RadioParams};
    use crate::replay::{load_info, write_sigmf_fixture};

    const NFM_META: &str = r#"{
        "global": {
            "core:datatype": "cf32_le",
            "core:sample_rate": 2048000,
            "core:version": "1.0.0",
            "core:description": "",
            "core:author": "RAIL",
            "rail:center_frequency_hz": 145500000,
            "rail:tuner_gain_db": 28.0,
            "rail:demod_mode": "NFM",
            "rail:filter_bandwidth_hz": 12500
        },
        "captures": [
            {
                "core:sample_start": 0,
                "core:datetime": "2026-09-07T12:00:00Z",
                "core:frequency": 145500000
            }
        ],
        "annotations": []
    }"#;

    const EXTERNAL_META: &str = r#"{
        "global": { "core:datatype": "cf32_le", "core:sample_rate": 2048000 },
        "captures": [{ "core:frequency": 145500000 }],
        "annotations": []
    }"#;

    /// AC#3: an NFM / 12.5 kHz SigMF file on disk replays through an
    /// NFM / 12.5 kHz chain. Covers the whole path `start_replay` takes
    /// — sidecar, plan, then the `DspParamState` the worker is seeded
    /// with — so restoring the chain's own WBFM defaults anywhere along
    /// it fails here.
    #[test]
    fn an_nfm_recording_replays_through_an_nfm_chain() {
        let data_path = write_sigmf_fixture("plan-nfm", NFM_META, 1_024);
        let plan = replay_plan(&load_info(&data_path).unwrap());

        assert_eq!(plan.sample_rate_hz, 2_048_000);
        let mut state = DspParamState::new(plan.sample_rate_hz as f32, plan.params);

        let config = state.chain_mut().config();
        assert_eq!(config.mode, DemodMode::Nfm);
        assert!((config.bandwidth_hz - 12_500.0).abs() < 0.5);
        assert_eq!(state.center_hz(), 145_500_000);
    }

    /// The frontend is told what the chain is actually running, not the
    /// "file did not say" sentinels: an external SigMF file replays
    /// through the defaults and the reply must say so.
    #[test]
    fn the_reply_reports_resolved_params_for_an_external_file() {
        let data_path = write_sigmf_fixture("reply-external", EXTERNAL_META, 1_024);
        let reply = ReplayInfoReply::from_info(&load_info(&data_path).unwrap());

        let defaults = RadioParams::default();
        assert_eq!(reply.demod_mode, defaults.mode_str());
        assert_eq!(reply.filter_bandwidth_hz, defaults.bandwidth_hz);
        assert!(!reply.demod_mode.is_empty());
    }

    /// The tag and the struct-variant field renaming only fail at
    /// runtime, so pin the exact JSON `src/ipc/commands.ts` sends.
    #[test]
    fn transport_args_deserialize_from_the_frontend_wire_shape() {
        let play: ReplayTransportArgs = serde_json::from_str(r#"{"kind":"play"}"#).unwrap();
        assert!(matches!(play, ReplayTransportArgs::Play));

        let pause: ReplayTransportArgs = serde_json::from_str(r#"{"kind":"pause"}"#).unwrap();
        assert!(matches!(pause, ReplayTransportArgs::Pause));

        let seek: ReplayTransportArgs =
            serde_json::from_str(r#"{"kind":"seek","positionMs":1500}"#).unwrap();
        assert!(matches!(
            seek,
            ReplayTransportArgs::Seek { position_ms: 1_500 }
        ));
    }
}
