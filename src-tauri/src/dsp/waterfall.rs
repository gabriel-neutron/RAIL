//! IQ byte conversion and waterfall frame assembly.
//!
//! See `docs/HARDWARE.md` §1 for the u8 → complex float formula
//! (`(byte / 127.5) − 1.0`, producing samples in `[-1.0, 1.0]`) and
//! `docs/DSP.md` §3 for the end-to-end waterfall pipeline.

use num_complex::Complex;

use crate::dsp::fft::FftProcessor;
use crate::error::RailError;

const BYTE_TO_FLOAT_SCALE: f32 = 1.0 / 127.5;

/// Convert interleaved RTL-SDR u8 samples (I,Q,I,Q,…) into complex floats.
///
/// `raw` length must be `2 × out.len()`. See `docs/HARDWARE.md` §1.
pub fn iq_u8_to_complex(raw: &[u8], out: &mut [Complex<f32>]) -> Result<(), RailError> {
    if raw.len() != out.len() * 2 {
        return Err(RailError::DspError(format!(
            "IQ length mismatch: {} bytes vs {} complex samples",
            raw.len(),
            out.len()
        )));
    }
    for (pair, dst) in raw.chunks_exact(2).zip(out.iter_mut()) {
        let i = pair[0] as f32 * BYTE_TO_FLOAT_SCALE - 1.0;
        let q = pair[1] as f32 * BYTE_TO_FLOAT_SCALE - 1.0;
        *dst = Complex::new(i, q);
    }
    Ok(())
}

/// One-shot frame builder: owns a scratch complex buffer matching the FFT
/// size and produces a full waterfall row per call.
pub struct FrameBuilder {
    fft: FftProcessor,
    iq: Vec<Complex<f32>>,
}

impl FrameBuilder {
    /// Allocate a frame builder for FFT size `n`.
    pub fn new(n: usize) -> Self {
        Self {
            fft: FftProcessor::new(n),
            iq: vec![Complex::new(0.0, 0.0); n],
        }
    }

    /// Number of complex samples (and output bins) per frame.
    pub fn size(&self) -> usize {
        self.fft.size()
    }

    /// Required raw-byte slice length per frame (2 × N for interleaved IQ).
    pub fn bytes_per_frame(&self) -> usize {
        self.fft.size() * 2
    }

    /// Convert one chunk of raw IQ bytes into a dB-scale, FFT-shifted
    /// spectrum. Returns a slice of length `size()`.
    pub fn build(&mut self, raw: &[u8]) -> Result<&[f32], RailError> {
        iq_u8_to_complex(raw, &mut self.iq)?;
        Ok(self.fft.process(&self.iq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_conversion_range() {
        let raw = [0u8, 127, 128, 255];
        let mut out = vec![Complex::new(0.0, 0.0); 2];
        iq_u8_to_complex(&raw, &mut out).unwrap();
        assert!((out[0].re - -1.0).abs() < 1e-6);
        assert!((out[0].im - (127.0 / 127.5 - 1.0)).abs() < 1e-6);
        assert!((out[1].re - (128.0 / 127.5 - 1.0)).abs() < 1e-6);
        assert!((out[1].im - (255.0 / 127.5 - 1.0)).abs() < 1e-6);
    }

    #[test]
    fn frame_builder_round_trip() {
        let mut fb = FrameBuilder::new(32);
        let raw = vec![128u8; fb.bytes_per_frame()];
        let frame = fb.build(&raw).unwrap();
        assert_eq!(frame.len(), 32);
    }

    #[test]
    fn size_mismatch_is_error() {
        let mut out = vec![Complex::new(0.0, 0.0); 4];
        let err = iq_u8_to_complex(&[0u8; 2], &mut out);
        assert!(err.is_err());
    }
}
