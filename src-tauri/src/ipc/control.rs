//! The single control seam between Tauri commands and the DSP worker.
//!
//! Every runtime parameter change — retune, mode, bandwidth, squelch,
//! gain, ppm, capture start/stop — travels as one [`DspControl`] message
//! on one unbounded channel, drained at one point in
//! [`super::dsp_task`]'s worker loop. Ordering between parameters is
//! therefore a property of the channel rather than of four independent
//! transports. See `docs/ARCHITECTURE.md` §3.
//!
//! Two halves live here:
//! * [`DspControlHandle`] — the command side. Owns the sender *and* the
//!   [`RadioParams`] state-of-record, updated on every send, so
//!   `radio_snapshot` reads current values locally and never has to
//!   query the worker.
//! * [`DspParamState`] — the worker side. Owns the [`DemodChain`] and the
//!   current centre frequency. Free of Tauri types on purpose so a
//!   control sequence can be asserted without a Tauri runtime.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::dsp::demod::{DemodChain, DemodConfig, DemodControl, DemodMode};
use crate::error::RailError;
use crate::ipc::capture_cmd::CaptureControl;

/// A runtime control message for the DSP worker.
///
/// `SetGainTenthsDb` and `SetPpm` carry no worker-side effect — the
/// hardware call stays in the command layer, which owns the tuner and
/// the synchronous validation errors — but they still travel the seam so
/// the state-of-record has a single writer.
pub(crate) enum DspControl {
    /// New centre frequency in Hz (already LO-offset corrected).
    Retune { center_hz: u32 },
    /// New demodulation mode.
    SetMode(DemodMode),
    /// New channel bandwidth in Hz.
    SetBandwidthHz(f32),
    /// Squelch threshold in dBFS; `None` disables the gate.
    SetSquelchDbfs(Option<f32>),
    /// Manual tuner gain in tenths of a dB; `None` means AGC.
    SetGainTenthsDb(Option<i32>),
    /// Frequency correction in parts per million.
    SetPpm(i32),
    /// Capture writer request (audio / IQ start and stop).
    Capture(CaptureControl),
}

/// The parameter state-of-record for a session.
///
/// Written only by [`DspControlHandle::send`], read by the capture
/// commands and by the worker seed. Replaces the shadow copies
/// `Session` used to keep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RadioParams {
    /// Tuned centre frequency in Hz.
    pub(crate) center_hz: u32,
    /// Current demodulation mode.
    pub(crate) mode: DemodMode,
    /// Channel bandwidth in Hz.
    pub(crate) bandwidth_hz: u32,
    /// Squelch threshold in dBFS; `None` when the gate is disabled.
    pub(crate) squelch_dbfs: Option<f32>,
    /// Manual tuner gain in tenths of a dB; `None` while in AGC.
    pub(crate) gain_tenths_db: Option<i32>,
    /// Frequency correction in parts per million.
    pub(crate) ppm: i32,
}

/// The one place a session's starting mode and bandwidth are decided.
///
/// Both values are read from [`DemodConfig::default`] rather than
/// restated, so the worker's chain and the state-of-record cannot drift
/// apart. `center_hz` has no meaningful default — every caller
/// overrides it with a tuned or recorded frequency.
impl Default for RadioParams {
    fn default() -> Self {
        let chain_defaults = DemodConfig::default();
        Self {
            center_hz: 0,
            mode: chain_defaults.mode,
            bandwidth_hz: chain_defaults.bandwidth_hz.max(0.0) as u32,
            squelch_dbfs: None,
            gain_tenths_db: None,
            ppm: 0,
        }
    }
}

impl RadioParams {
    /// Fold one control message into the state-of-record. Messages with
    /// no parameter payload (capture requests) are a no-op.
    pub(crate) fn apply(&mut self, msg: &DspControl) {
        match msg {
            DspControl::Retune { center_hz } => self.center_hz = *center_hz,
            DspControl::SetMode(mode) => self.mode = *mode,
            DspControl::SetBandwidthHz(hz) => self.bandwidth_hz = hz.max(0.0) as u32,
            DspControl::SetSquelchDbfs(db) => self.squelch_dbfs = *db,
            DspControl::SetGainTenthsDb(tenths) => self.gain_tenths_db = *tenths,
            DspControl::SetPpm(ppm) => self.ppm = *ppm,
            DspControl::Capture(_) => {}
        }
    }

    /// Wire name of the current mode. Kept in sync with `parse_mode` in
    /// [`super::commands`] and `src/store/radio.ts :: DemodMode`.
    pub(crate) fn mode_str(&self) -> &'static str {
        match self.mode {
            DemodMode::Fm => "FM",
            DemodMode::Nfm => "NFM",
            DemodMode::Am => "AM",
            DemodMode::Usb => "USB",
            DemodMode::Lsb => "LSB",
            DemodMode::Cw => "CW",
        }
    }
}

/// Command-side half of the control seam: one sender plus the shared
/// state-of-record.
///
/// [`send`](Self::send) updates [`RadioParams`] *before* forwarding, so a
/// snapshot taken the moment a command returns is already current. That
/// is what lets the capture commands stay local: a paused replay or a
/// dead dongle stops the worker draining, but no command waits on it.
#[derive(Clone)]
pub(crate) struct DspControlHandle {
    tx: mpsc::UnboundedSender<DspControl>,
    params: Arc<Mutex<RadioParams>>,
}

impl DspControlHandle {
    /// Build a handle and the receiver the DSP worker drains.
    /// `initial` seeds both the state-of-record and the worker.
    pub(crate) fn new(initial: RadioParams) -> (Self, mpsc::UnboundedReceiver<DspControl>) {
        let (tx, rx) = mpsc::unbounded_channel::<DspControl>();
        (
            Self {
                tx,
                params: Arc::new(Mutex::new(initial)),
            },
            rx,
        )
    }

    /// Record `msg` in the state-of-record, then forward it to the worker.
    /// The send is unbounded and never blocks.
    pub(crate) fn send(&self, msg: DspControl) -> Result<(), RailError> {
        {
            let mut params = self.params.lock().map_err(params_poisoned)?;
            params.apply(&msg);
        }
        self.tx
            .send(msg)
            .map_err(|e| RailError::StreamError(format!("dsp control channel closed: {e}")))
    }

    /// Current parameter state-of-record.
    pub(crate) fn snapshot(&self) -> Result<RadioParams, RailError> {
        let params = self.params.lock().map_err(params_poisoned)?;
        Ok(*params)
    }
}

fn params_poisoned<T>(_: std::sync::PoisonError<T>) -> RailError {
    RailError::StreamError("radio params lock poisoned".into())
}

/// What the worker must do beyond applying the message itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ControlEffect {
    /// Discard every accumulated spectral sample: the centre frequency
    /// moved, so buffered samples belong to a different frequency.
    pub(crate) flush_spectral: bool,
}

/// Worker-side half of the control seam: the parameters the DSP task
/// owns. Free of Tauri types so control sequences are testable.
pub(crate) struct DspParamState {
    chain: DemodChain,
    center_hz: u32,
}

impl DspParamState {
    /// Build the worker state for an IQ input rate, seeded from the
    /// session's initial [`RadioParams`].
    pub(crate) fn new(input_rate_hz: f32, initial: RadioParams) -> Self {
        let config = DemodConfig {
            mode: initial.mode,
            bandwidth_hz: initial.bandwidth_hz as f32,
            squelch_dbfs: squelch_for_chain(initial.squelch_dbfs),
        };
        Self {
            chain: DemodChain::with_config(input_rate_hz, config),
            center_hz: initial.center_hz,
        }
    }

    /// Apply one control message and report what else the worker owes.
    pub(crate) fn apply(&mut self, msg: DspControl) -> ControlEffect {
        match msg {
            DspControl::Retune { center_hz } => {
                let changed = center_hz != self.center_hz;
                self.center_hz = center_hz;
                ControlEffect {
                    flush_spectral: changed,
                }
            }
            DspControl::SetMode(mode) => {
                self.chain.apply(DemodControl::SetMode(mode));
                ControlEffect::default()
            }
            DspControl::SetBandwidthHz(hz) => {
                self.chain.apply(DemodControl::SetBandwidthHz(hz));
                ControlEffect::default()
            }
            DspControl::SetSquelchDbfs(db) => {
                self.chain
                    .apply(DemodControl::SetSquelchDbfs(squelch_for_chain(db)));
                ControlEffect::default()
            }
            // Gain and ppm act on the tuner, which the command layer owns.
            // They cross the seam only to keep one writer for the snapshot.
            DspControl::SetGainTenthsDb(_) | DspControl::SetPpm(_) => ControlEffect::default(),
            // Capture requests are handled by the worker's writer state,
            // not by the parameter state.
            DspControl::Capture(_) => ControlEffect::default(),
        }
    }

    /// Current centre frequency in Hz.
    pub(crate) fn center_hz(&self) -> u32 {
        self.center_hz
    }

    /// Mutable access to the demodulator chain for the audio path.
    pub(crate) fn chain_mut(&mut self) -> &mut DemodChain {
        &mut self.chain
    }
}

/// Translate the seam's `Option` squelch into the `DemodChain`'s
/// sentinel representation. The `f32::NEG_INFINITY` "off" value exists
/// only inside the chain: `serde_json` writes non-finite floats as
/// `null` and refuses to read them back into a bare `f32`, so anything
/// that reaches SigMF metadata must stay an `Option`.
fn squelch_for_chain(db: Option<f32>) -> f32 {
    db.filter(|v| v.is_finite()).unwrap_or(f32::NEG_INFINITY)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use tokio::sync::oneshot;

    const TEST_INPUT_RATE_HZ: f32 = 2_048_000.0;

    fn params_at(center_hz: u32) -> RadioParams {
        RadioParams {
            center_hz,
            mode: DemodMode::Fm,
            bandwidth_hz: 200_000,
            squelch_dbfs: None,
            gain_tenths_db: None,
            ppm: 0,
        }
    }

    #[test]
    fn control_sequence_produces_expected_worker_state() {
        let mut state = DspParamState::new(TEST_INPUT_RATE_HZ, params_at(100_000_000));
        let sequence = vec![
            DspControl::SetMode(DemodMode::Am),
            DspControl::SetBandwidthHz(15_000.0),
            DspControl::Retune {
                center_hz: 101_100_000,
            },
            DspControl::SetSquelchDbfs(Some(-60.0)),
        ];

        let effects: Vec<ControlEffect> = sequence.into_iter().map(|m| state.apply(m)).collect();

        assert_eq!(state.center_hz(), 101_100_000);
        let config = state.chain_mut().config();
        assert_eq!(config.mode, DemodMode::Am);
        assert!((config.bandwidth_hz - 15_000.0).abs() < 0.5);
        assert!((config.squelch_dbfs + 60.0).abs() < 1e-6);

        let flushes: Vec<bool> = effects.iter().map(|e| e.flush_spectral).collect();
        assert_eq!(
            flushes,
            vec![false, false, true, false],
            "only a centre-changing retune may flush the accumulator"
        );

        // A retune to the same frequency is not a flush.
        let effect = state.apply(DspControl::Retune {
            center_hz: 101_100_000,
        });
        assert!(!effect.flush_spectral);
    }

    #[test]
    fn handle_send_updates_params_before_forwarding() {
        let (handle, mut rx) = DspControlHandle::new(params_at(100_000_000));

        handle
            .send(DspControl::Retune {
                center_hz: 144_500_000,
            })
            .unwrap();
        // Nothing has drained the channel yet, and the snapshot is already current.
        let snap = handle.snapshot().unwrap();
        assert_eq!(snap.center_hz, 144_500_000);

        handle.send(DspControl::SetMode(DemodMode::Nfm)).unwrap();
        handle.send(DspControl::SetBandwidthHz(12_500.0)).unwrap();
        handle
            .send(DspControl::SetSquelchDbfs(Some(-45.0)))
            .unwrap();
        handle.send(DspControl::SetGainTenthsDb(Some(297))).unwrap();
        handle.send(DspControl::SetPpm(-3)).unwrap();

        let snap = handle.snapshot().unwrap();
        assert_eq!(snap.mode, DemodMode::Nfm);
        assert_eq!(snap.mode_str(), "NFM");
        assert_eq!(snap.bandwidth_hz, 12_500);
        assert_eq!(snap.squelch_dbfs, Some(-45.0));
        assert_eq!(snap.gain_tenths_db, Some(297));
        assert_eq!(snap.ppm, -3);

        // Every message still reached the worker, in order.
        let mut drained = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            drained.push(msg);
        }
        assert_eq!(drained.len(), 6);
        assert!(matches!(
            drained[0],
            DspControl::Retune {
                center_hz: 144_500_000
            }
        ));
        assert!(matches!(drained[1], DspControl::SetMode(DemodMode::Nfm)));
    }

    #[test]
    fn squelch_none_maps_to_neg_infinity_at_the_chain() {
        let mut state = DspParamState::new(TEST_INPUT_RATE_HZ, params_at(100_000_000));
        state.apply(DspControl::SetSquelchDbfs(Some(-70.0)));
        assert!((state.chain_mut().config().squelch_dbfs + 70.0).abs() < 1e-6);

        state.apply(DspControl::SetSquelchDbfs(None));
        assert_eq!(state.chain_mut().config().squelch_dbfs, f32::NEG_INFINITY);

        // The sentinel never leaves the chain: the seam keeps `None`, which
        // serde round-trips; a bare non-finite f32 does not.
        let (handle, _rx) = DspControlHandle::new(params_at(100_000_000));
        handle.send(DspControl::SetSquelchDbfs(None)).unwrap();
        let off = handle.snapshot().unwrap().squelch_dbfs;
        assert_eq!(off, None);
        let json = serde_json::to_string(&off).unwrap();
        assert_eq!(json, "null");
        assert_eq!(serde_json::from_str::<Option<f32>>(&json).unwrap(), None);
        // What the seam avoids: a non-finite f32 serializes to `null` and
        // will not deserialize back into a bare `f32`.
        let sentinel = serde_json::to_string(&f32::NEG_INFINITY).unwrap();
        assert_eq!(sentinel, "null");
        assert!(serde_json::from_str::<f32>(&sentinel).is_err());
    }

    #[test]
    fn capture_variant_is_a_no_op_for_params() {
        let mut state = DspParamState::new(TEST_INPUT_RATE_HZ, params_at(100_000_000));
        let before = state.chain_mut().config();

        let (reply, _reply_rx) = oneshot::channel();
        let effect = state.apply(DspControl::Capture(CaptureControl::StopAudio { reply }));
        assert!(!effect.flush_spectral);
        assert_eq!(state.center_hz(), 100_000_000);
        let after = state.chain_mut().config();
        assert_eq!(before.mode, after.mode);
        assert!((before.bandwidth_hz - after.bandwidth_hz).abs() < f32::EPSILON);

        let (handle, _rx) = DspControlHandle::new(params_at(100_000_000));
        let unchanged = handle.snapshot().unwrap();
        let (reply, _reply_rx) = oneshot::channel();
        handle
            .send(DspControl::Capture(CaptureControl::StopAudio { reply }))
            .unwrap();
        assert_eq!(handle.snapshot().unwrap(), unchanged);
    }
}
