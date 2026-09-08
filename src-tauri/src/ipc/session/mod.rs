//! Session lifetime: one assembly path shared by live and replay.
//!
//! [`types`] holds the state a running session owns, [`start`] is the
//! single place a session is built and installed, and [`live`] adds the
//! RTL-SDR-specific opening policy on top of it. Replay's counterpart
//! lives in [`super::replay_cmd`].
//!
//! See `docs/ARCHITECTURE.md` §3.

pub mod live;
pub mod start;
pub mod types;

pub(crate) use start::{ensure_idle, start_session, ScanAccumulator, SessionPlan};
pub(crate) use types::{session_poisoned, AppState, ReplayBits, SessionSource};
