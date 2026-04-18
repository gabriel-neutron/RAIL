//! Tagged input for the DSP task.
//!
//! The DSP worker consumes IQ from either the live RTL-SDR reader
//! ([`crate::hardware::stream::IqStream`]) or from a SigMF replay
//! ([`crate::replay`]). Both share the same processing chain; they
//! only differ in *how* the samples arrive.
//!
//! - Live: raw interleaved `u8` straight from librtlsdr. The DSP
//!   task has to convert them to complex and apply the `fs/4` LO
//!   mixer (see `docs/HARDWARE.md` §1 and `docs/DSP.md` §1).
//! - Replay: pre-shifted `cf32` samples read straight from a
//!   `.sigmf-data` file. They were produced by the SigMF writer
//!   downstream of the `fs/4` mixer, so the DSP task must skip
//!   both conversion and the shift for these.

use num_complex::Complex;

/// One chunk of IQ delivered to the DSP task.
pub enum DspInput {
    /// Raw RTL-SDR output: interleaved `I, Q, I, Q, …` bytes.
    RtlU8(Vec<u8>),
    /// Already-shifted complex samples from a SigMF replay.
    Cf32Shifted(Vec<Complex<f32>>),
    /// Exactly-one-FFT-window of shifted samples used to backfill the
    /// waterfall on seek. The DSP task FFTs this chunk into a single
    /// waterfall row and emits it immediately — no demod, no audio,
    /// and the rate limiter that normally throttles the emit stream
    /// is bypassed so a burst of prefill rows isn't dropped.
    ///
    /// The caller is responsible for sending `FFT_SIZE` samples.
    Cf32Prefill(Vec<Complex<f32>>),
}
