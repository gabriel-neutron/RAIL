//! What a running session owns, and the global slot it lives in.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::error::RailError;
use crate::hardware::stream::IqStream;
use crate::hardware::RtlSdrTuner;
use crate::ipc::control::DspControlHandle;
use crate::replay::{ReplayControl, ReplayInfo};

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
    pub(crate) stream: Option<IqStream>,
    /// Thread-safe tuning surface. `None` after the session is torn down
    /// so that late `set_gain` calls return an error instead of racing
    /// with device close.
    pub(crate) tuner: Option<RtlSdrTuner>,
    /// Discrete gain steps the hardware supports (tenths of dB).
    pub(crate) gains: Vec<i32>,
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

/// Global, single-session state.
#[derive(Default)]
pub struct AppState {
    pub(crate) session: Mutex<Option<Session>>,
    /// Active scanner task, if any. Held separately from `session` to
    /// avoid deadlocks between the scanner and command handlers.
    pub(crate) scanner: Mutex<Option<crate::scanner::ScannerHandle>>,
}

/// Map a poisoned `session` mutex onto a reportable error.
pub(crate) fn session_poisoned<T>(_: std::sync::PoisonError<T>) -> RailError {
    RailError::StreamError("session lock poisoned".into())
}
