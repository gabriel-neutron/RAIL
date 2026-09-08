//! The single session-assembly path.
//!
//! Live and replay differ only in how IQ samples reach the DSP worker.
//! Everything else — the control seam, the worker spawn, the install
//! into [`AppState`] and the `device-status` emit — happens here once,
//! so a session's starting parameters reach the chain by construction
//! rather than by each caller remembering to send them.

use std::sync::{Arc, Mutex};

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::mpsc;

use crate::dsp::input::DspInput;
use crate::error::RailError;
use crate::hardware::stream::{IqCanceler, IQ_CHANNEL_CAPACITY};
use crate::ipc::control::{DspControlHandle, RadioParams};
use crate::ipc::dsp_task::{spawn_dsp_task, DspTaskCfg, FFT_SIZE};
use crate::ipc::event_contract::Emit;
use crate::ipc::events::DeviceStatus;
use crate::ipc::session::types::{session_poisoned, AppState, Session, SessionSource};

/// Reject a start request when a session is already installed.
/// `busy_msg` is the user-facing wording for the calling command.
pub(crate) fn ensure_idle(state: &State<'_, AppState>, busy_msg: &str) -> Result<(), RailError> {
    let guard = state.session.lock().map_err(session_poisoned)?;
    if guard.is_some() {
        return Err(RailError::InvalidParameter(busy_msg.into()));
    }
    Ok(())
}

/// How much per-bin peak storage a session needs.
///
/// Only live sessions can be scanned, so replay allocates nothing —
/// the DSP task writes into a buffer no scanner will ever read.
pub(crate) enum ScanAccumulator {
    /// One `-inf` slot per FFT bin, ready for the scanner.
    Sized,
    /// Empty: this session cannot be scanned.
    Empty,
}

impl ScanAccumulator {
    fn allocate(&self) -> Vec<f32> {
        match self {
            Self::Sized => vec![f32::NEG_INFINITY; FFT_SIZE],
            Self::Empty => Vec::new(),
        }
    }
}

/// Everything [`start_session`] needs that is not source-specific.
pub(crate) struct SessionPlan {
    /// IQ sample rate in Hz feeding the DSP worker.
    pub(crate) sample_rate_hz: u32,
    /// Starting parameters — seeded into both the state-of-record and
    /// the worker's [`crate::ipc::control::DspParamState`].
    pub(crate) params: RadioParams,
    pub(crate) scan_accumulator: ScanAccumulator,
}

/// Build a session, install it, and announce it.
///
/// `attach` receives the IQ sender and wires up whichever source is
/// starting, returning the source-specific state plus the canceler the
/// worker uses to stop a hardware reader (`None` for replay, which
/// stops when its reader drops the sender). It runs before the worker
/// spawns because a live reader must exist for its canceler to be
/// handed over.
///
/// The session is installed only after the control seam is seeded, so
/// the capture commands never observe a session whose `control`
/// snapshot would misreport the mode or bandwidth.
pub(crate) fn start_session<R, F>(
    app: &AppHandle<R>,
    state: &State<'_, AppState>,
    plan: SessionPlan,
    waterfall_channel: Channel<InvokeResponseBody>,
    audio_channel: Channel<InvokeResponseBody>,
    attach: F,
) -> Result<(), RailError>
where
    R: Runtime,
    F: FnOnce(mpsc::Sender<DspInput>) -> Result<(SessionSource, Option<IqCanceler>), RailError>,
{
    let (iq_tx, iq_rx) = mpsc::channel::<DspInput>(IQ_CHANNEL_CAPACITY);
    let (control, control_rx) = DspControlHandle::new(plan.params);

    let (source, canceler) = attach(iq_tx)?;

    let max_dbfs_per_bin = Arc::new(Mutex::new(plan.scan_accumulator.allocate()));
    let dsp_handle = spawn_dsp_task(DspTaskCfg {
        app: app.clone(),
        iq_rx,
        waterfall_channel,
        audio_channel,
        control_rx,
        canceler,
        sample_rate_hz: plan.sample_rate_hz,
        initial_params: control.snapshot()?,
        max_dbfs_per_bin: max_dbfs_per_bin.clone(),
    });

    let mut guard = state.session.lock().map_err(session_poisoned)?;
    *guard = Some(Session {
        dsp: Some(dsp_handle),
        sample_rate_hz: plan.sample_rate_hz,
        control,
        source,
        max_dbfs_per_bin,
    });
    drop(guard);

    let _ = DeviceStatus::connected().emit(app);
    Ok(())
}
