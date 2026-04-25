//! DSP pipeline: FFT, windowing, magnitude/dB, demodulation.
//!
//! Math lives in `docs/DSP.md`. Do not duplicate derivations in code.

pub mod classifier;
pub mod demod;
pub mod fft;
pub mod filter;
pub mod input;
pub mod waterfall;
