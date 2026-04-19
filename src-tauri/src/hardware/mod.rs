//! RTL-SDR hardware access.
//!
//! See `docs/HARDWARE.md` for librtlsdr sequencing and tuning constraints.
//!
//! # Binding choice (Phase 1)
//!
//! RAIL binds to a **system `librtlsdr`** through hand-written `extern "C"`
//! declarations in [`ffi`]. Rationale:
//!
//! - The public API is tiny and stable (~14 symbols), so `bindgen` + libclang
//!   is not worth the build-time cost.
//! - Linking at build time (via `build.rs` and `LIBRTLSDR_LIB_DIR`) keeps
//!   startup fast and surfaces missing-driver errors at link time, not at
//!   first sample.
//! - The DLL/SO installation is a one-time developer step documented in
//!   `docs/TECH_STACK.md` §4. Windows users additionally need Zadig to
//!   replace the default driver with WinUSB (see `docs/HARDWARE.md` §6).
//!
//! The `nusb`-based [`check_device`] is kept as a fast, driver-free
//! "dongle is physically attached" probe used by the UI on startup. Actual
//! tuning/streaming requires [`RtlSdrDevice::open`] and therefore librtlsdr.

use std::ffi::{c_int, c_void, CStr};
use std::ptr;

use serde::Serialize;

use crate::error::RailError;

pub mod ffi;
pub mod stream;

/// Serializable RTL-SDR device description sent to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
}

/// Known RTL2832U-based dongle USB identifiers (vendor_id, product_id).
/// Reference: rtl-sdr.com hardware list. NESDR Smart v5 = `0x0bda:0x2838`.
const KNOWN_RTLSDR_IDS: &[(u16, u16)] = &[
    (0x0bda, 0x2832), // Generic RTL2832U
    (0x0bda, 0x2838), // Generic RTL2832U / NESDR Smart
    (0x0bda, 0x2831), // Realtek RTL2831U
    (0x0bda, 0x2834), // Realtek variants
    (0x0ccd, 0x00a9), // Terratec Cinergy T Stick Black
    (0x0ccd, 0x00b3), // Terratec NOXON DAB/DAB+
    (0x185b, 0x0620), // Compro Videomate U620F
    (0x1f4d, 0xa803), // SVEON STV20
    (0x1554, 0x5020), // PixelView PV-DT235U
];

fn is_rtlsdr(vid: u16, pid: u16) -> bool {
    KNOWN_RTLSDR_IDS.iter().any(|&(v, p)| v == vid && p == pid)
}

fn friendly_name(vid: u16, pid: u16, product: Option<&str>) -> String {
    if let Some(p) = product.filter(|s| !s.is_empty()) {
        return p.to_string();
    }
    match (vid, pid) {
        (0x0bda, 0x2838) => "Generic RTL2832U / NESDR".into(),
        (0x0bda, 0x2832) => "Generic RTL2832U".into(),
        (vid, pid) => format!("RTL-SDR compatible ({vid:04x}:{pid:04x})"),
    }
}

/// Enumerate attached RTL-SDR compatible USB devices via `nusb`.
///
/// Returns the first matching device, or `RailError::DeviceNotFound` if none
/// are attached. Does **not** open the device — opening is done in
/// [`RtlSdrDevice::open`] during stream startup.
pub fn check_device() -> Result<DeviceInfo, RailError> {
    let devices = nusb::list_devices()
        .map_err(|e| RailError::DeviceOpenFailed(format!("USB enumeration failed: {e}")))?;

    for (idx, dev) in devices.enumerate() {
        let vid = dev.vendor_id();
        let pid = dev.product_id();
        if is_rtlsdr(vid, pid) {
            let name = friendly_name(vid, pid, dev.product_string());
            log::info!("RTL-SDR found at USB index {idx}: {name} ({vid:04x}:{pid:04x})");
            return Ok(DeviceInfo {
                index: idx as u32,
                name,
            });
        }
    }

    Err(RailError::DeviceNotFound)
}

/// Return the number of RTL-SDR devices visible to librtlsdr (post-driver).
/// Differs from [`check_device`]: this one requires the WinUSB/udev driver
/// to be installed and will return 0 otherwise.
pub fn librtlsdr_device_count() -> u32 {
    // SAFETY: `rtlsdr_get_device_count` takes no arguments and is
    // thread-safe per the librtlsdr source.
    unsafe { ffi::rtlsdr_get_device_count() }
}

/// Safe RAII handle around a `librtlsdr` device.
///
/// The raw pointer is moved to a single worker thread for streaming (see
/// `stream.rs`); therefore we assert `Send`. We do **not** implement `Sync`
/// because librtlsdr's device APIs are not reentrant.
pub struct RtlSdrDevice {
    ptr: *mut ffi::RtlSdrDev,
}

// SAFETY: the handle is only ever touched from one thread at a time
// (see `docs/HARDWARE.md` §2). We transfer ownership across threads but
// never share it concurrently.
unsafe impl Send for RtlSdrDevice {}

impl RtlSdrDevice {
    /// Open the device at `index`. Fails with `DeviceOpenFailed` if
    /// librtlsdr returns a non-zero status (driver missing, device busy).
    pub fn open(index: u32) -> Result<Self, RailError> {
        let mut ptr: *mut ffi::RtlSdrDev = ptr::null_mut();
        // SAFETY: `ptr` is a valid mutable location. librtlsdr writes the
        // new handle into it on success.
        let rc = unsafe { ffi::rtlsdr_open(&mut ptr, index) };
        if rc != 0 || ptr.is_null() {
            return Err(RailError::DeviceOpenFailed(format!(
                "rtlsdr_open({index}) -> {rc}"
            )));
        }
        Ok(Self { ptr })
    }

    /// Human-readable device name for the given index.
    pub fn device_name(index: u32) -> Option<String> {
        // SAFETY: librtlsdr returns either NULL or a pointer to a static
        // string living for the duration of the process.
        let raw = unsafe { ffi::rtlsdr_get_device_name(index) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: pointer is non-null and points to a NUL-terminated static.
        let cstr = unsafe { CStr::from_ptr(raw) };
        cstr.to_str().ok().map(str::to_owned)
    }

    /// Configure sample rate in Hz. See `docs/HARDWARE.md` §4 for stable
    /// rates (we default to 2 048 000 in stream setup).
    pub fn set_sample_rate(&self, hz: u32) -> Result<(), RailError> {
        // SAFETY: `self.ptr` is a valid librtlsdr handle owned by `self`.
        let rc = unsafe { ffi::rtlsdr_set_sample_rate(self.ptr, hz) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_sample_rate({hz}) -> {rc}"
            )));
        }
        Ok(())
    }

    /// Set tuner center frequency in Hz.
    pub fn set_center_freq(&self, hz: u32) -> Result<(), RailError> {
        // SAFETY: handle owned by self.
        let rc = unsafe { ffi::rtlsdr_set_center_freq(self.ptr, hz) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_center_freq({hz}) -> {rc}"
            )));
        }
        Ok(())
    }

    /// Read back the actual tuned frequency (may differ from the request —
    /// see `docs/HARDWARE.md` §4).
    pub fn center_freq(&self) -> u32 {
        // SAFETY: handle owned by self.
        unsafe { ffi::rtlsdr_get_center_freq(self.ptr) }
    }

    /// `true` = manual gain mode, `false` = hardware AGC.
    pub fn set_tuner_gain_mode(&self, manual: bool) -> Result<(), RailError> {
        let flag: c_int = if manual { 1 } else { 0 };
        // SAFETY: handle owned by self.
        let rc = unsafe { ffi::rtlsdr_set_tuner_gain_mode(self.ptr, flag) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_tuner_gain_mode({manual}) -> {rc}"
            )));
        }
        Ok(())
    }

    /// Gain in tenths of a dB (librtlsdr's native unit). Only meaningful
    /// when manual gain mode is on.
    pub fn set_tuner_gain_tenths(&self, tenths_db: i32) -> Result<(), RailError> {
        // SAFETY: handle owned by self.
        let rc = unsafe { ffi::rtlsdr_set_tuner_gain(self.ptr, tenths_db) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_tuner_gain({tenths_db}) -> {rc}"
            )));
        }
        Ok(())
    }

    /// Discrete gain steps supported by the tuner (tenths of dB).
    pub fn available_gains(&self) -> Result<Vec<i32>, RailError> {
        // First call with NULL gets the count.
        // SAFETY: NULL is the librtlsdr-documented sentinel to request the
        // count without filling a buffer.
        let count = unsafe { ffi::rtlsdr_get_tuner_gains(self.ptr, ptr::null_mut()) };
        if count < 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_get_tuner_gains(NULL) -> {count}"
            )));
        }
        let count = count as usize;
        if count == 0 {
            return Ok(Vec::new());
        }

        let mut buf = vec![0i32; count];
        // SAFETY: buf has `count` capacity and `len == count`. librtlsdr
        // writes exactly `count` ints.
        let written = unsafe { ffi::rtlsdr_get_tuner_gains(self.ptr, buf.as_mut_ptr()) };
        if written < 0 || written as usize != count {
            return Err(RailError::StreamError(format!(
                "rtlsdr_get_tuner_gains(buf) -> {written}"
            )));
        }
        Ok(buf)
    }

    /// PPM crystal correction (see `docs/HARDWARE.md` §3).
    pub fn set_freq_correction_ppm(&self, ppm: i32) -> Result<(), RailError> {
        // librtlsdr returns -2 when the correction is unchanged; treat as OK.
        // SAFETY: handle owned by self.
        let rc = unsafe { ffi::rtlsdr_set_freq_correction(self.ptr, ppm) };
        if rc != 0 && rc != -2 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_freq_correction({ppm}) -> {rc}"
            )));
        }
        Ok(())
    }

    /// Must be called once before `read_async`. See `docs/HARDWARE.md` §6
    /// (skipping `reset_buffer` is the #1 cause of "callback not called").
    pub fn reset_buffer(&self) -> Result<(), RailError> {
        // SAFETY: handle owned by self.
        let rc = unsafe { ffi::rtlsdr_reset_buffer(self.ptr) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_reset_buffer -> {rc}"
            )));
        }
        Ok(())
    }

    /// Start the blocking async read loop. `cb` will be invoked on
    /// librtlsdr's internal thread until [`Self::cancel_async`] is called.
    ///
    /// # Safety
    ///
    /// The caller must ensure `ctx` outlives every invocation of `cb`, and
    /// that `cb` does not unwind across the FFI boundary. The stream task
    /// in `stream.rs` enforces both.
    pub unsafe fn read_async(
        &self,
        cb: ffi::ReadAsyncCb,
        ctx: *mut c_void,
        buf_num: u32,
        buf_len: u32,
    ) -> Result<(), RailError> {
        let rc = ffi::rtlsdr_read_async(self.ptr, cb, ctx, buf_num, buf_len);
        if rc != 0 {
            return Err(RailError::StreamError(format!("rtlsdr_read_async -> {rc}")));
        }
        Ok(())
    }

    /// Signal the async loop to exit. Safe to call from any thread.
    pub fn cancel_async(&self) -> Result<(), RailError> {
        // SAFETY: librtlsdr documents `rtlsdr_cancel_async` as safe to call
        // from a different thread than the one running `read_async`.
        let rc = unsafe { ffi::rtlsdr_cancel_async(self.ptr) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_cancel_async -> {rc}"
            )));
        }
        Ok(())
    }

    /// Expose the raw pointer for `rtlsdr_cancel_async` to be called from a
    /// thread that only holds a weak reference. Only used by
    /// [`stream::IqStream`].
    pub(crate) fn as_ptr(&self) -> *mut ffi::RtlSdrDev {
        self.ptr
    }

    /// Clone a [`TunerHandle`] that can be used to retune or change gain
    /// from threads other than the reader thread. See [`TunerHandle`] for
    /// the thread-safety assumptions.
    pub fn tuner_handle(&self) -> TunerHandle {
        TunerHandle { ptr: self.ptr }
    }
}

/// Thread-safe control surface for tuning/gain changes during streaming.
///
/// Does **not** own the device — the reader thread does. The owner
/// guarantees the device stays open for at least as long as any
/// [`TunerHandle`] that calls into it (see the lifecycle management in
/// `commands::Session`).
///
/// librtlsdr's `rtlsdr_set_center_freq`, `rtlsdr_set_tuner_gain_mode`,
/// and `rtlsdr_set_tuner_gain` are documented as callable while a
/// `read_async` loop is running on another thread; this is the standard
/// pattern used by `rtl_fm` and every SDR UI on top of librtlsdr.
#[derive(Clone, Copy)]
pub struct TunerHandle {
    ptr: *mut ffi::RtlSdrDev,
}

// SAFETY: only `set_center_freq`, `set_tuner_gain_mode`, `set_tuner_gain`
// are reachable through this type — all documented as thread-safe vs the
// reader thread.
unsafe impl Send for TunerHandle {}
unsafe impl Sync for TunerHandle {}

impl TunerHandle {
    pub fn set_center_freq(&self, hz: u32) -> Result<(), RailError> {
        // SAFETY: see type-level doc — caller guarantees the underlying
        // device is still open.
        let rc = unsafe { ffi::rtlsdr_set_center_freq(self.ptr, hz) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_center_freq({hz}) -> {rc}"
            )));
        }
        Ok(())
    }

    pub fn set_tuner_gain_mode(&self, manual: bool) -> Result<(), RailError> {
        let flag: c_int = if manual { 1 } else { 0 };
        // SAFETY: as above.
        let rc = unsafe { ffi::rtlsdr_set_tuner_gain_mode(self.ptr, flag) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_tuner_gain_mode({manual}) -> {rc}"
            )));
        }
        Ok(())
    }

    pub fn set_tuner_gain_tenths(&self, tenths_db: i32) -> Result<(), RailError> {
        // SAFETY: as above.
        let rc = unsafe { ffi::rtlsdr_set_tuner_gain(self.ptr, tenths_db) };
        if rc != 0 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_tuner_gain({tenths_db}) -> {rc}"
            )));
        }
        Ok(())
    }

    /// Read back the currently tuned center frequency. librtlsdr snaps the
    /// requested Hz to the tuner's resolution; the UI uses this value to
    /// reflect what actually happened after [`Self::set_center_freq`].
    pub fn center_freq(&self) -> u32 {
        // SAFETY: see type-level doc.
        unsafe { ffi::rtlsdr_get_center_freq(self.ptr) }
    }

    /// PPM crystal correction while streaming. See `docs/HARDWARE.md` §3.
    ///
    /// librtlsdr returns `-2` when the requested value matches the current
    /// one — treated as success.
    pub fn set_freq_correction_ppm(&self, ppm: i32) -> Result<(), RailError> {
        // SAFETY: see type-level doc.
        let rc = unsafe { ffi::rtlsdr_set_freq_correction(self.ptr, ppm) };
        if rc != 0 && rc != -2 {
            return Err(RailError::StreamError(format!(
                "rtlsdr_set_freq_correction({ppm}) -> {rc}"
            )));
        }
        Ok(())
    }
}

impl Drop for RtlSdrDevice {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: we own the handle and no other thread is using it at
            // this point (Drop requires `&mut self`).
            let _ = unsafe { ffi::rtlsdr_close(self.ptr) };
            self.ptr = ptr::null_mut();
        }
    }
}
