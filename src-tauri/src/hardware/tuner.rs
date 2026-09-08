//! The tuner port — the seam between tuning logic and librtlsdr.
//!
//! [`Tuner`] is a thin mirror of the librtlsdr control calls RAIL makes while
//! a stream is running. Two adapters implement it: [`RtlSdrTuner`] in
//! production, and `fake_tuner::FakeTuner` under `cfg(test)` so the scanner
//! sweep can be exercised with no dongle attached.
//!
//! The port deliberately does **not** absorb the `fs/4` LO offset (see
//! `docs/DSP.md` §1): every call site keeps its own `target − fs/4` in /
//! `actual + fs/4` out arithmetic, because a mistake hidden inside the
//! adapter would be a silent, dongle-only failure.

use std::ffi::c_int;

use crate::error::RailError;
use crate::hardware::ffi;

/// Control surface for tuning and gain changes on a live SDR.
///
/// Implementors must be callable from a thread other than the one running the
/// IQ read loop; see `docs/HARDWARE.md` §2 for librtlsdr's reentrancy rules
/// and `docs/ARCHITECTURE.md` §4 for the threading model.
pub trait Tuner: Send + Sync {
    /// Set the tuner centre frequency, in Hz. See `docs/HARDWARE.md` §4 for
    /// the supported range and the tuner's frequency resolution.
    fn set_center_freq(&self, hz: u32) -> Result<(), RailError>;

    /// Read back the currently tuned centre frequency, in Hz. The hardware
    /// snaps the requested value to the tuner's resolution, so this may differ
    /// from what [`Tuner::set_center_freq`] was given (`docs/HARDWARE.md` §4).
    fn center_freq(&self) -> u32;

    /// Select the gain source: `true` = manual gain, `false` = hardware AGC.
    /// Must select manual *before* a gain value is written
    /// (`docs/HARDWARE.md` §3).
    fn set_tuner_gain_mode(&self, manual: bool) -> Result<(), RailError>;

    /// Set the manual gain, in tenths of a dB (librtlsdr's native unit). Only
    /// meaningful once [`Tuner::set_tuner_gain_mode`] has selected manual
    /// mode. Supported steps are enumerated at open time by
    /// [`crate::hardware::RtlSdrDevice::available_gains`].
    fn set_tuner_gain_tenths(&self, tenths_db: i32) -> Result<(), RailError>;

    /// Set the crystal correction, in ppm. See `docs/HARDWARE.md` §3.
    ///
    /// librtlsdr answers `-2` when the requested value already matches the
    /// current one; implementors treat that as success, not as an error.
    fn set_freq_correction_ppm(&self, ppm: i32) -> Result<(), RailError>;
}

/// Production [`Tuner`] adapter over a `librtlsdr` device handle.
///
/// Does **not** own the device — the reader thread does (see
/// [`crate::hardware::stream::IqStream`]). The owner guarantees the device
/// stays open for at least as long as any `RtlSdrTuner` that calls into it:
/// `stop_stream` awaits the scanner task before tearing the session down, and
/// clears the session's tuner so late commands fail instead of racing the
/// close.
///
/// librtlsdr documents these control calls as usable while a `read_async` loop
/// runs on another thread; this is the pattern `rtl_fm` and every SDR UI built
/// on librtlsdr uses.
#[derive(Clone, Copy)]
pub struct RtlSdrTuner {
    ptr: *mut ffi::RtlSdrDev,
}

// SAFETY: exactly five librtlsdr calls are reachable through this type —
// `rtlsdr_set_center_freq`, `rtlsdr_get_center_freq`,
// `rtlsdr_set_tuner_gain_mode`, `rtlsdr_set_tuner_gain` and
// `rtlsdr_set_freq_correction` — all documented as callable from a thread
// other than the one running `read_async`. The pointer is not owned here, so
// soundness additionally rests on the device outliving every copy of this
// handle: `ipc::commands::stop_stream` awaits the scanner task before closing
// the device, and that task is the only place a copy escapes.
unsafe impl Send for RtlSdrTuner {}
// SAFETY: as above — the reachable calls are thread-safe against the reader
// thread, and the lifetime argument is the same.
unsafe impl Sync for RtlSdrTuner {}

impl RtlSdrTuner {
    /// Wrap a live `librtlsdr` handle. The caller keeps ownership of the
    /// device and must keep it open for as long as this handle is reachable.
    pub(crate) fn from_ptr(ptr: *mut ffi::RtlSdrDev) -> Self {
        Self { ptr }
    }
}

impl Tuner for RtlSdrTuner {
    fn set_center_freq(&self, hz: u32) -> Result<(), RailError> {
        // SAFETY: see the type-level doc — the caller guarantees the
        // underlying device is still open.
        let rc = unsafe { ffi::rtlsdr_set_center_freq(self.ptr, hz) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_center_freq({hz}) -> {rc}"
            )));
        }
        Ok(())
    }

    fn center_freq(&self) -> u32 {
        // SAFETY: see the type-level doc.
        unsafe { ffi::rtlsdr_get_center_freq(self.ptr) }
    }

    fn set_tuner_gain_mode(&self, manual: bool) -> Result<(), RailError> {
        let flag: c_int = if manual { 1 } else { 0 };
        // SAFETY: see the type-level doc.
        let rc = unsafe { ffi::rtlsdr_set_tuner_gain_mode(self.ptr, flag) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_tuner_gain_mode({manual}) -> {rc}"
            )));
        }
        Ok(())
    }

    fn set_tuner_gain_tenths(&self, tenths_db: i32) -> Result<(), RailError> {
        // SAFETY: see the type-level doc.
        let rc = unsafe { ffi::rtlsdr_set_tuner_gain(self.ptr, tenths_db) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_tuner_gain({tenths_db}) -> {rc}"
            )));
        }
        Ok(())
    }

    fn set_freq_correction_ppm(&self, ppm: i32) -> Result<(), RailError> {
        // librtlsdr returns -2 when the correction is unchanged; treat as OK.
        // SAFETY: see the type-level doc.
        let rc = unsafe { ffi::rtlsdr_set_freq_correction(self.ptr, ppm) };
        if rc != 0 && rc != -2 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_freq_correction({ppm}) -> {rc}"
            )));
        }
        Ok(())
    }
}

/// Lets a borrowed tuner stand in for an owned one, so a caller can keep the
/// adapter (and its recorded state, in tests) while handing the port to the
/// sweep.
impl<T: Tuner + ?Sized> Tuner for &T {
    fn set_center_freq(&self, hz: u32) -> Result<(), RailError> {
        (**self).set_center_freq(hz)
    }

    fn center_freq(&self) -> u32 {
        (**self).center_freq()
    }

    fn set_tuner_gain_mode(&self, manual: bool) -> Result<(), RailError> {
        (**self).set_tuner_gain_mode(manual)
    }

    fn set_tuner_gain_tenths(&self, tenths_db: i32) -> Result<(), RailError> {
        (**self).set_tuner_gain_tenths(tenths_db)
    }

    fn set_freq_correction_ppm(&self, ppm: i32) -> Result<(), RailError> {
        (**self).set_freq_correction_ppm(ppm)
    }
}
