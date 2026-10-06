//! Raw `extern "C"` bindings to a system `librtlsdr`.
//!
//! Hand-written against the stable librtlsdr public header
//! (<https://github.com/osmocom/rtl-sdr/blob/master/include/rtl-sdr.h>).
//! We only bind the ~14 symbols Phase 1 needs. See `docs/HARDWARE.md` §2
//! for the expected open/configure/stream sequence.
//!
//! Every function in this file is `unsafe`; the safe wrappers live in
//! [`super::mod`](super). Keeping the raw bindings isolated here means the
//! rest of the crate never touches raw pointers or the C ABI directly.

use std::ffi::{c_char, c_int, c_uchar, c_uint, c_void};

/// Opaque handle returned by `rtlsdr_open`. We never dereference it from
/// Rust; librtlsdr treats it as an incomplete struct internally.
#[repr(C)]
pub struct RtlSdrDev {
    _private: [u8; 0],
}

/// Async read callback invoked by librtlsdr on its own thread.
/// `buf` points to `len` interleaved u8 samples (I, Q, I, Q, ...).
/// `ctx` is the opaque pointer we registered in `rtlsdr_read_async`.
pub type ReadAsyncCb = extern "C" fn(buf: *mut c_uchar, len: c_uint, ctx: *mut c_void);

extern "C" {
    /// Number of RTL-SDR devices attached to the system.
    pub fn rtlsdr_get_device_count() -> c_uint;

    /// Borrowed, NUL-terminated display name for `index`, or null when the index is out of range. Owned by librtlsdr; never freed by the caller.
    pub fn rtlsdr_get_device_name(index: c_uint) -> *const c_char;

    /// Open device `index`, writing a newly owned handle into `dev`. Returns 0 on success, negative on failure; a successful handle must be released with `rtlsdr_close`.
    pub fn rtlsdr_open(dev: *mut *mut RtlSdrDev, index: c_uint) -> c_int;

    /// Close a handle obtained from `rtlsdr_open` and release it. Returns 0 on success, negative on failure.
    pub fn rtlsdr_close(dev: *mut RtlSdrDev) -> c_int;

    /// Set the IQ sample rate in Hz. Returns 0 on success, negative on failure.
    pub fn rtlsdr_set_sample_rate(dev: *mut RtlSdrDev, rate: c_uint) -> c_int;

    /// Set the tuner centre frequency in Hz. Returns 0 on success, negative on failure.
    pub fn rtlsdr_set_center_freq(dev: *mut RtlSdrDev, freq: c_uint) -> c_int;

    /// Current tuner centre frequency in Hz, or 0 on failure.
    pub fn rtlsdr_get_center_freq(dev: *mut RtlSdrDev) -> c_uint;

    /// Select manual gain (`manual` non-zero) or the tuner AGC (0). Returns 0 on success, negative on failure.
    pub fn rtlsdr_set_tuner_gain_mode(dev: *mut RtlSdrDev, manual: c_int) -> c_int;

    /// Set the manual tuner gain in tenths of a dB. Returns 0 on success, negative on failure.
    pub fn rtlsdr_set_tuner_gain(dev: *mut RtlSdrDev, gain_tenths_db: c_int) -> c_int;

    /// Return the number of supported gains, and fill `gains` with that many values in tenths of a dB when it is non-null. The buffer is caller-owned and must hold at least that many `c_int`; pass null first to size it. Negative on failure.
    pub fn rtlsdr_get_tuner_gains(dev: *mut RtlSdrDev, gains: *mut c_int) -> c_int;

    /// Set the crystal frequency correction in parts per million. Returns 0 on success, negative on failure.
    pub fn rtlsdr_set_freq_correction(dev: *mut RtlSdrDev, ppm: c_int) -> c_int;

    /// Discard buffered samples before starting a stream. Returns 0 on success, negative on failure.
    pub fn rtlsdr_reset_buffer(dev: *mut RtlSdrDev) -> c_int;

    /// Run the async read loop, invoking `cb` with `ctx` per buffer, using `buf_num` buffers of `buf_len` bytes. Blocks the calling thread until `rtlsdr_cancel_async`. Returns 0 on success, negative on failure.
    pub fn rtlsdr_read_async(
        dev: *mut RtlSdrDev,
        cb: ReadAsyncCb,
        ctx: *mut c_void,
        buf_num: c_uint,
        buf_len: c_uint,
    ) -> c_int;

    /// Ask a running `rtlsdr_read_async` loop to return. Returns 0 on success, negative on failure.
    pub fn rtlsdr_cancel_async(dev: *mut RtlSdrDev) -> c_int;
}
