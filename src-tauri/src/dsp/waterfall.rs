//! IQ byte conversion and waterfall frame assembly.
//!
//! See `docs/HARDWARE.md` §1 for the u8 → complex float formula
//! (`(byte / 127.5) − 1.0`, producing samples in `[-1.0, 1.0]`) and
//! `docs/DSP.md` §3 for the end-to-end waterfall pipeline.
//!
//! # DC offset mitigation
//!
//! When enabled (see [`FrameBuilder::set_lo_offset_enabled`]), the builder
//! applies an `exp(-j·π·n/2)` mixer before the FFT. Paired with an LO that
//! is parked `fs/4` above the target frequency, this shifts the signal of
//! interest down to 0 Hz (canvas center) while the hardware DC spike ends
//! up at `-fs/4` — off the center bin. See `docs/DSP.md` §1 and §8.

use num_complex::Complex;

use crate::dsp::fft::FftProcessor;
use crate::error::RailError;

const BYTE_TO_FLOAT_SCALE: f32 = 1.0 / 127.5;

/// Convert interleaved RTL-SDR u8 samples (I,Q,I,Q,…) into complex floats.
///
/// `raw` length must be `2 × out.len()`. See `docs/HARDWARE.md` §1.
///
/// librtlsdr already compensates for the R820T IF spectrum inversion
/// inside the RTL2832U demod, so the IQ stream reaching us is upright —
/// no software conjugation is needed here.
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

/// Apply an in-place `exp(-j·π·n/2)` mixer starting at phase index
/// `phase_idx` (mod 4). Returns the next phase index to continue with.
///
/// This is a trivial `fs/4` downconversion — no trig, just sign/swap
/// cycling through `[1, -j, -1, +j]`.
///
/// Exposed `pub` so the DSP task can shift once per IQ chunk and fan
/// the result out to both the FFT frame builder and the demod chain.
pub fn apply_fs4_shift(samples: &mut [Complex<f32>], phase_idx: u32) -> u32 {
    let mut k = (phase_idx & 0b11) as usize;
    for s in samples.iter_mut() {
        let re = s.re;
        let im = s.im;
        *s = match k {
            0 => Complex::new(re, im),
            1 => Complex::new(im, -re),
            2 => Complex::new(-re, -im),
            _ => Complex::new(-im, re),
        };
        k = (k + 1) & 0b11;
    }
    k as u32
}

/// One-shot frame builder: owns a scratch complex buffer matching the FFT
/// size and produces a full waterfall row per call.
pub struct FrameBuilder {
    fft: FftProcessor,
    iq: Vec<Complex<f32>>,
    lo_offset_enabled: bool,
    phase_idx: u32,
}

impl FrameBuilder {
    /// Allocate a frame builder for FFT size `n`. The `fs/4` LO-offset
    /// mixer is enabled by default.
    pub fn new(n: usize) -> Self {
        Self {
            fft: FftProcessor::new(n),
            iq: vec![Complex::new(0.0, 0.0); n],
            lo_offset_enabled: true,
            phase_idx: 0,
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

    /// Toggle the `fs/4` digital mixer. Disabling is intended for tests
    /// and for hardware that handles the DC offset itself.
    pub fn set_lo_offset_enabled(&mut self, enabled: bool) {
        self.lo_offset_enabled = enabled;
    }

    /// Convert one chunk of raw IQ bytes into a dB-scale, FFT-shifted
    /// spectrum. Returns a slice of length `size()`.
    pub fn build(&mut self, raw: &[u8]) -> Result<&[f32], RailError> {
        iq_u8_to_complex(raw, &mut self.iq)?;
        if self.lo_offset_enabled {
            self.phase_idx = apply_fs4_shift(&mut self.iq, self.phase_idx);
        }
        Ok(self.fft.process(&self.iq))
    }

    /// Run only the FFT stage on IQ samples that were already converted
    /// and (if needed) `fs/4`-shifted upstream. Lets the DSP task share
    /// one shifted buffer between the waterfall and demod chains.
    pub fn process_shifted(&mut self, iq: &[Complex<f32>]) -> Result<&[f32], RailError> {
        if iq.len() != self.iq.len() {
            return Err(RailError::DspError(format!(
                "FFT input length mismatch: {} vs {}",
                iq.len(),
                self.iq.len()
            )));
        }
        Ok(self.fft.process(iq))
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
        fb.set_lo_offset_enabled(false);
        let raw = vec![128u8; fb.bytes_per_frame()];
        let frame = fb.build(&raw).unwrap();
        assert_eq!(frame.len(), 32);
    }

    #[test]
    fn fs4_shift_cycles_correctly() {
        let mut samples = vec![
            Complex::new(1.0_f32, 0.0),
            Complex::new(1.0, 0.0),
            Complex::new(1.0, 0.0),
            Complex::new(1.0, 0.0),
        ];
        let next = apply_fs4_shift(&mut samples, 0);
        assert_eq!(next, 0, "phase wraps back to 0 after 4 samples");
        assert!((samples[0].re - 1.0).abs() < 1e-6 && samples[0].im.abs() < 1e-6);
        assert!(samples[1].re.abs() < 1e-6 && (samples[1].im + 1.0).abs() < 1e-6);
        assert!((samples[2].re + 1.0).abs() < 1e-6 && samples[2].im.abs() < 1e-6);
        assert!(samples[3].re.abs() < 1e-6 && (samples[3].im - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fs4_shift_moves_dc_off_center() {
        // DC input (constant real) after fs/4 downshift must peak at the
        // `n/4`-from-center bin (docs/DSP.md §1).
        let n = 64;
        let mut iq = vec![Complex::new(1.0_f32, 0.0); n];
        apply_fs4_shift(&mut iq, 0);
        let mut fft = crate::dsp::fft::FftProcessor::new(n);
        let spectrum = fft.process(&iq);
        let peak_bin = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        // DC was at bin n/2 after fft_shift. The fs/4 downshift moves it
        // to bin n/2 - n/4 = n/4.
        assert_eq!(peak_bin, n / 4);
    }

    #[test]
    fn size_mismatch_is_error() {
        let mut out = vec![Complex::new(0.0, 0.0); 4];
        let err = iq_u8_to_complex(&[0u8; 2], &mut out);
        assert!(err.is_err());
    }

    /// Full-pipeline round-trip against a synthetic upright-R820T signal.
    ///
    /// librtlsdr returns upright IQ (α = +1). Simulates a carrier at
    /// baseband offset `u = +600/2048 · fs` above the user-tuned center
    /// `fc`, with LO parked at `fc − fs/4` (δ = −fs/4). Builds the IQ
    /// bytes the hardware would emit, runs them through
    /// `iq_u8_to_complex` → `apply_fs4_shift` → FFT → `fft_shift`, and
    /// asserts the peak lands at bin `N/2 + u_bins` (correct display),
    /// not at the mirror bin `N/2 − u_bins`.
    #[test]
    fn pipeline_round_trip_upright_tuner() {
        const N: usize = 2048;
        const U_BINS: i32 = 600;
        const DELTA_BINS: i32 = -(N as i32) / 4; // -512

        // Analog-mixer output frequency relative to fs: +1·(u − δ) = 1112.
        let analog_bin = (U_BINS - DELTA_BINS) as f32;
        let two_pi = 2.0 * std::f32::consts::PI;

        let mut raw = Vec::<u8>::with_capacity(2 * N);
        for k in 0..N {
            let phase = two_pi * analog_bin * (k as f32) / (N as f32);
            let (sin, cos) = phase.sin_cos();
            let i_byte = (((cos + 1.0) * 127.5).round() as i32).clamp(0, 255) as u8;
            let q_byte = (((sin + 1.0) * 127.5).round() as i32).clamp(0, 255) as u8;
            raw.push(i_byte);
            raw.push(q_byte);
        }

        let mut iq = vec![Complex::new(0.0_f32, 0.0); N];
        iq_u8_to_complex(&raw, &mut iq).unwrap();
        apply_fs4_shift(&mut iq, 0);
        let mut fft = crate::dsp::fft::FftProcessor::new(N);
        let spectrum = fft.process(&iq);
        let peak_bin = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();

        let correct_bin = ((N as i32) / 2 + U_BINS) as usize;
        let mirror_bin = ((N as i32) / 2 - U_BINS) as usize;
        assert_eq!(
            peak_bin, correct_bin,
            "upright-tuner pipeline should put the signal at bin {correct_bin}, got {peak_bin} \
             (mirror would be {mirror_bin})"
        );
    }
}
