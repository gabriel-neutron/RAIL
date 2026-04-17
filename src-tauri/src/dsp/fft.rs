//! FFT pipeline wrapper around `rustfft`.
//!
//! See `docs/DSP.md` §2–3 for the full magnitude/dB/shift definitions.

use std::sync::Arc;

use num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::dsp::filter::hann_window;

/// Noise-floor clamp below which dB values are pinned, preventing
/// `-inf` when a bin magnitude is zero.
const DB_FLOOR: f32 = -200.0;

/// Forward-FFT processor that computes a windowed magnitude-dB spectrum
/// with a centered DC bin (FFT-shifted).
pub struct FftProcessor {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    out_db: Vec<f32>,
    n: usize,
    norm: f32,
}

impl FftProcessor {
    /// Plan a forward FFT of size `n` (must be > 1; powers of two are
    /// fastest — see `docs/DSP.md` §2).
    pub fn new(n: usize) -> Self {
        assert!(n > 1, "FFT size must be > 1");
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            fft,
            window: hann_window(n),
            buffer: vec![Complex::new(0.0, 0.0); n],
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            out_db: vec![DB_FLOOR; n],
            n,
            norm: n as f32,
        }
    }

    /// FFT size.
    pub fn size(&self) -> usize {
        self.n
    }

    /// Run the full pipeline on exactly `n` complex samples and return
    /// the dB-scaled, FFT-shifted magnitude spectrum.
    ///
    /// See `docs/DSP.md` §2 for the per-step rationale.
    pub fn process(&mut self, iq: &[Complex<f32>]) -> &[f32] {
        debug_assert_eq!(iq.len(), self.n, "FFT input length mismatch");

        for (dst, (&src, &w)) in self.buffer.iter_mut().zip(iq.iter().zip(self.window.iter())) {
            *dst = src * w;
        }

        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);

        for (dst, src) in self.out_db.iter_mut().zip(self.buffer.iter()) {
            let mag = src.norm() / self.norm;
            *dst = if mag > 0.0 {
                20.0 * mag.log10()
            } else {
                DB_FLOOR
            };
        }

        fft_shift(&mut self.out_db);
        &self.out_db
    }
}

/// Swap the two halves of `data` in place so bin 0 (DC) lands at the
/// center. See `docs/DSP.md` §2 step 6.
pub fn fft_shift(data: &mut [f32]) {
    let n = data.len();
    let half = n / 2;
    data.rotate_left(half);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_input_peaks_at_center_after_shift() {
        let mut proc = FftProcessor::new(64);
        let iq: Vec<Complex<f32>> = (0..64).map(|_| Complex::new(1.0, 0.0)).collect();
        let spectrum = proc.process(&iq);
        let peak_bin = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(peak_bin, 32, "DC should land at n/2 after fft_shift");
    }

    #[test]
    fn shift_is_rotation() {
        let mut data = vec![1.0_f32, 2.0, 3.0, 4.0];
        fft_shift(&mut data);
        assert_eq!(data, vec![3.0, 4.0, 1.0, 2.0]);
    }
}
