//! Test-only [`Tuner`] adapter.
//!
//! Split out of `tuner.rs` to keep both files under the 300-line cap in
//! `docs/CONVENTIONS.md` §1. Compiled only under `cfg(test)`, so it can never
//! become a silent no-hardware demo path in the shipped binary.

use crate::error::RailError;
use crate::hardware::tuner::Tuner;

/// One recorded call on a [`FakeTuner`], in the order it was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TunerCall {
    /// [`Tuner::set_center_freq`] with the requested Hz.
    SetCenterFreq(u32),
    /// [`Tuner::center_freq`] read-back.
    CenterFreq,
    /// [`Tuner::set_tuner_gain_mode`] with `manual`.
    SetGainMode(bool),
    /// [`Tuner::set_tuner_gain_tenths`] with the requested tenths of a dB.
    SetGainTenths(i32),
    /// [`Tuner::set_freq_correction_ppm`] with the requested ppm.
    SetPpm(i32),
}

#[derive(Default)]
struct FakeState {
    calls: Vec<TunerCall>,
    tuned_hz: Vec<u32>,
    gain_mode: Vec<bool>,
    gain_tenths: Vec<i32>,
    ppm: Vec<i32>,
    last_tuned_hz: u32,
}

/// In-memory [`Tuner`] adapter for tests. Records every call so sweep
/// sequencing can be asserted with no dongle attached.
///
/// Test-only by construction (`cfg(test)`), so it can never become a silent
/// no-hardware demo path in the shipped binary.
pub(crate) struct FakeTuner {
    state: std::sync::Mutex<FakeState>,
    /// Frequency (as passed to [`Tuner::set_center_freq`]) that fails with a
    /// `StreamError`, to make the retune-failure branch reachable.
    fail_tune_at: Option<u32>,
    /// Grid the read-back snaps the last tuned frequency down to, mimicking
    /// the tuner's frequency resolution (`docs/HARDWARE.md` §4). Keeps
    /// `center_freq` from merely echoing the request. `0` disables snapping.
    read_back_snap_hz: u32,
}

impl FakeTuner {
    /// A fake that accepts every tune and snaps read-back down to `snap_hz`.
    pub(crate) fn new(read_back_snap_hz: u32) -> Self {
        Self {
            state: std::sync::Mutex::new(FakeState::default()),
            fail_tune_at: None,
            read_back_snap_hz,
        }
    }

    /// As [`FakeTuner::new`], but every tune to `hz` returns an error.
    pub(crate) fn failing_at(read_back_snap_hz: u32, hz: u32) -> Self {
        Self {
            state: std::sync::Mutex::new(FakeState::default()),
            fail_tune_at: Some(hz),
            read_back_snap_hz,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every frequency handed to [`Tuner::set_center_freq`], in order,
    /// including the ones that failed.
    pub(crate) fn tuned_hz(&self) -> Vec<u32> {
        self.state().tuned_hz.clone()
    }

    /// The recorded gain-mode flags and gain values, each in call order.
    pub(crate) fn gain_calls(&self) -> (Vec<bool>, Vec<i32>) {
        let s = self.state();
        (s.gain_mode.clone(), s.gain_tenths.clone())
    }

    /// Every ppm value handed to [`Tuner::set_freq_correction_ppm`].
    pub(crate) fn ppm_calls(&self) -> Vec<i32> {
        self.state().ppm.clone()
    }

    /// The full ordered call log.
    pub(crate) fn calls(&self) -> Vec<TunerCall> {
        self.state().calls.clone()
    }
}

impl Tuner for FakeTuner {
    fn set_center_freq(&self, hz: u32) -> Result<(), RailError> {
        let mut s = self.state();
        s.calls.push(TunerCall::SetCenterFreq(hz));
        s.tuned_hz.push(hz);
        if self.fail_tune_at == Some(hz) {
            return Err(RailError::StreamError(format!(
                "fake: rtlsdr_set_center_freq({hz}) -> -1"
            )));
        }
        s.last_tuned_hz = hz;
        Ok(())
    }

    fn center_freq(&self) -> u32 {
        let mut s = self.state();
        s.calls.push(TunerCall::CenterFreq);
        let last = s.last_tuned_hz;
        if self.read_back_snap_hz == 0 {
            last
        } else {
            last - (last % self.read_back_snap_hz)
        }
    }

    fn set_tuner_gain_mode(&self, manual: bool) -> Result<(), RailError> {
        let mut s = self.state();
        s.calls.push(TunerCall::SetGainMode(manual));
        s.gain_mode.push(manual);
        Ok(())
    }

    fn set_tuner_gain_tenths(&self, tenths_db: i32) -> Result<(), RailError> {
        let mut s = self.state();
        s.calls.push(TunerCall::SetGainTenths(tenths_db));
        s.gain_tenths.push(tenths_db);
        Ok(())
    }

    fn set_freq_correction_ppm(&self, ppm: i32) -> Result<(), RailError> {
        let mut s = self.state();
        s.calls.push(TunerCall::SetPpm(ppm));
        s.ppm.push(ppm);
        Ok(())
    }
}

mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::{FakeTuner, TunerCall};
    use crate::hardware::tuner::Tuner;

    #[test]
    fn fake_records_calls_in_order_and_snaps_read_back() {
        let tuner = FakeTuner::new(1_000);
        tuner.set_tuner_gain_mode(true).unwrap();
        tuner.set_tuner_gain_tenths(496).unwrap();
        tuner.set_freq_correction_ppm(-2).unwrap();
        tuner.set_center_freq(100_000_123).unwrap();

        // Read-back deliberately differs from the request: the real tuner
        // snaps to its frequency resolution (docs/HARDWARE.md §4).
        assert_eq!(tuner.center_freq(), 100_000_000);
        assert_eq!(tuner.tuned_hz(), vec![100_000_123]);
        assert_eq!(tuner.gain_calls(), (vec![true], vec![496]));
        assert_eq!(tuner.ppm_calls(), vec![-2]);
        assert_eq!(
            tuner.calls(),
            vec![
                TunerCall::SetGainMode(true),
                TunerCall::SetGainTenths(496),
                TunerCall::SetPpm(-2),
                TunerCall::SetCenterFreq(100_000_123),
                TunerCall::CenterFreq,
            ]
        );
    }

    #[test]
    fn fake_fails_only_the_configured_frequency() {
        let tuner = FakeTuner::failing_at(0, 90_000_000);
        assert!(tuner.set_center_freq(90_000_000).is_err());
        assert!(tuner.set_center_freq(90_200_000).is_ok());
        // The failed tune is still recorded, and did not move the read-back.
        assert_eq!(tuner.tuned_hz(), vec![90_000_000, 90_200_000]);
        assert_eq!(tuner.center_freq(), 90_200_000);
    }
}
