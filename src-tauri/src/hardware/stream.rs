//! IQ sample reader for the RTL-SDR async stream.
//!
//! See `docs/ARCHITECTURE.md` §4 (threading model) and `docs/HARDWARE.md` §2
//! (initialization sequence). librtlsdr's `rtlsdr_read_async` blocks until
//! cancelled, so we run it on a dedicated `std::thread`. The C callback
//! fires on that same thread; we copy the raw u8 buffer into a
//! `tokio::sync::mpsc` channel for the DSP task to consume.

use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use tokio::sync::mpsc;

use crate::error::RailError;
use crate::hardware::{ffi, RtlSdrDevice};

/// Ring buffer depth for the stream → DSP handoff, in frames.
/// See `docs/ARCHITECTURE.md` §4 ("8 frames minimum to absorb USB jitter").
pub const IQ_CHANNEL_CAPACITY: usize = 8;

/// Default per-buffer size in bytes. 32 768 bytes at fs = 2.048 MHz ≈ 8 ms.
/// See `docs/HARDWARE.md` §3.
pub const DEFAULT_USB_BUF_LEN: u32 = 32_768;

/// Number of librtlsdr USB buffers. 4 is the stock rtl_sdr.c value and
/// keeps the double-buffering overhead reasonable.
pub const DEFAULT_USB_BUF_NUM: u32 = 4;

/// Skip this many initial callbacks to avoid the startup click/pop
/// documented in `docs/HARDWARE.md` §6.
pub const STARTUP_SKIP_BUFFERS: usize = 2;

/// State shared with the C callback. Kept on the reader thread's stack,
/// so its lifetime always exceeds any callback invocation.
struct CbCtx {
    tx: mpsc::Sender<Vec<u8>>,
    skip_remaining: AtomicUsize,
    dropped: AtomicU64,
}

/// Entry point for librtlsdr's async reader. Runs on librtlsdr's own thread.
///
/// Safety contract:
/// - `ctx` was supplied to `rtlsdr_read_async` by [`IqStream::start`] and
///   points to a `CbCtx` that outlives this call.
/// - `buf` points to `len` valid bytes owned by librtlsdr for the call's
///   duration; we copy them immediately and do not retain the pointer.
extern "C" fn on_iq(buf: *mut u8, len: u32, ctx: *mut c_void) {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if ctx.is_null() || buf.is_null() || len == 0 {
            return;
        }
        // SAFETY: see function docstring above.
        let ctx_ref = unsafe { &*(ctx as *const CbCtx) };

        let skip = ctx_ref.skip_remaining.load(Ordering::Relaxed);
        if skip > 0 {
            ctx_ref.skip_remaining.store(skip - 1, Ordering::Relaxed);
            return;
        }

        // SAFETY: librtlsdr guarantees `buf` points to `len` valid bytes.
        let slice = unsafe { std::slice::from_raw_parts(buf, len as usize) };
        let owned = slice.to_vec();

        if ctx_ref.tx.try_send(owned).is_err() {
            let dropped = ctx_ref.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if dropped.is_power_of_two() {
                log::warn!("IQ buffer full, dropped {dropped} frames so far");
            }
        }
    }));
    if result.is_err() {
        log::error!("panic caught in IQ callback (suppressed across FFI)");
    }
}

/// Thread-safe wrapper around the raw device pointer, exposed only so
/// other threads can call `rtlsdr_cancel_async` (documented thread-safe
/// in librtlsdr). The device itself is owned by the reader thread.
///
/// The `requested` flag distinguishes a deliberate shutdown from an
/// unexpected exit (e.g. the dongle was physically unplugged). The
/// reader thread uses it to decide whether to close the handle — on
/// Windows WinUSB, `rtlsdr_close` on a disconnected device segfaults.
struct Canceler {
    ptr: *mut ffi::RtlSdrDev,
    requested: AtomicBool,
}

// SAFETY: `rtlsdr_cancel_async` is the only call we make through this
// pointer and librtlsdr documents it as safe from any thread.
unsafe impl Send for Canceler {}
unsafe impl Sync for Canceler {}

impl Canceler {
    fn cancel(&self) {
        self.requested.store(true, Ordering::SeqCst);
        // SAFETY: ptr is valid until the reader thread's `read_async`
        // returns; we only call this before joining the thread.
        unsafe {
            let _ = ffi::rtlsdr_cancel_async(self.ptr);
        }
    }

    fn was_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

/// Callback invoked from the reader thread when `rtlsdr_read_async`
/// exits without a matching `cancel()` — i.e. the device disappeared
/// (unplugged, driver restart, bus reset). Runs on the reader thread;
/// must not block.
pub type OnDisconnect = Box<dyn FnOnce(String) + Send + 'static>;

/// Public, clonable handle that other tasks can use to request the
/// reader thread to stop (e.g. the DSP task when its frontend channel
/// dies). Calling `cancel` is idempotent and safe from any thread.
#[derive(Clone)]
pub struct IqCanceler(Arc<Canceler>);

impl IqCanceler {
    pub fn cancel(&self) {
        self.0.cancel();
    }
}

/// Handle for a running async IQ stream. Dropping it cancels and joins.
pub struct IqStream {
    canceler: Arc<Canceler>,
    thread: Option<JoinHandle<Result<(), RailError>>>,
}

impl IqStream {
    /// Spawn the reader thread for an already-configured device.
    ///
    /// The device is moved into the worker thread and closed when the
    /// thread exits cleanly. `tx` is the producer side of the IQ channel
    /// consumed by the DSP task. `on_disconnect` fires on the reader
    /// thread if `read_async` returns without a matching [`Self::stop`]
    /// — the caller uses it to notify the frontend.
    pub fn start(
        device: RtlSdrDevice,
        tx: mpsc::Sender<Vec<u8>>,
        buf_num: u32,
        buf_len: u32,
        on_disconnect: OnDisconnect,
    ) -> Result<Self, RailError> {
        device.reset_buffer()?;
        let canceler = Arc::new(Canceler {
            ptr: device.as_ptr(),
            requested: AtomicBool::new(false),
        });
        let canceler_for_thread = canceler.clone();

        let thread = thread::Builder::new()
            .name("rail-iq-reader".into())
            .spawn(move || -> Result<(), RailError> {
                let ctx = CbCtx {
                    tx,
                    skip_remaining: AtomicUsize::new(STARTUP_SKIP_BUFFERS),
                    dropped: AtomicU64::new(0),
                };
                let ctx_ptr: *mut c_void = (&ctx as *const CbCtx) as *mut c_void;
                // SAFETY: `ctx` lives on this function's stack until
                // `read_async` returns, so any callback invocation sees a
                // valid context. `on_iq` is `extern "C"` and panic-safe.
                let rc = unsafe { device.read_async(on_iq, ctx_ptr, buf_num, buf_len) };

                // `read_async` returning without us having asked for it
                // means librtlsdr bailed out internally — on Windows this
                // is nearly always a USB disconnect (see the
                // `cb transfer status: 4/5, canceling...` pattern). The
                // `rc` itself is unreliable as a disconnect signal because
                // librtlsdr can return 0 even after every transfer failed;
                // `was_requested` is the ground truth.
                if canceler_for_thread.was_requested() {
                    drop(device);
                } else {
                    let msg = match &rc {
                        Ok(()) => "device stream stopped unexpectedly".to_string(),
                        Err(e) => e.to_string(),
                    };
                    log::warn!(
                        "rtlsdr_read_async exited unexpectedly ({msg}); \
                         leaking handle to avoid rtlsdr_close segfault on \
                         disconnected WinUSB device"
                    );
                    // Closing a disconnected WinUSB handle segfaults
                    // librtlsdr on Windows. The allocation is tiny and
                    // only lives until process exit — a worthwhile trade.
                    std::mem::forget(device);
                    on_disconnect(msg);
                }

                rc
            })
            .map_err(|e| RailError::StreamError(format!("spawn stream thread: {e}")))?;

        Ok(Self {
            canceler,
            thread: Some(thread),
        })
    }

    /// Return a cloneable handle that can cancel the reader from any
    /// thread. Used by the DSP task to stop the pipeline when its
    /// frontend channel disconnects.
    pub fn canceler(&self) -> IqCanceler {
        IqCanceler(self.canceler.clone())
    }

    /// Cancel the async loop and join the reader thread.
    pub fn stop(mut self) -> Result<(), RailError> {
        self.canceler.cancel();
        if let Some(handle) = self.thread.take() {
            match handle.join() {
                Ok(res) => res?,
                Err(_) => {
                    return Err(RailError::StreamError(
                        "IQ reader thread panicked".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Drop for IqStream {
    fn drop(&mut self) {
        // Only cancel+join if `stop()` was not already called. Cancelling
        // twice is a use-after-free because `stop()` joins the reader
        // thread, which closes the device via `RtlSdrDevice::drop`.
        if let Some(handle) = self.thread.take() {
            self.canceler.cancel();
            let _ = handle.join();
        }
    }
}
