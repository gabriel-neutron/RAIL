//! Capture subsystem: streaming WAV + SigMF writers and a tiny
//! tmp-file helper. Audio and IQ recordings stream from the DSP task
//! straight to disk (see `docs/SIGNALS.md` §1–2) so the frontend only
//! deals in paths, not large byte buffers.

pub mod sigmf;
pub mod tmp;
pub mod wav;
