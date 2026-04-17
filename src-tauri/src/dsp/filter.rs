//! Windowing and (future) decimation filters.
//!
//! See `docs/DSP.md` §7. Phase 1 only uses the Hann window; the
//! windowed-sinc low-pass filter arrives with the Phase 3 demod chain.

use std::f32::consts::PI;

/// N-point Hann window: `w[n] = 0.5 · (1 − cos(2π·n / (N−1)))`.
///
/// See `docs/DSP.md` §7. Default choice for the waterfall FFT (medium
/// side-lobe suppression, narrow main lobe).
pub fn hann_window(n: usize) -> Vec<f32> {
    if n < 2 {
        return vec![1.0; n];
    }
    let denom = (n - 1) as f32;
    (0..n)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / denom).cos()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_edges_are_zero() {
        let w = hann_window(8);
        assert!(w[0].abs() < 1e-6);
        assert!(w[7].abs() < 1e-6);
    }

    #[test]
    fn hann_symmetric() {
        let w = hann_window(16);
        for i in 0..8 {
            assert!((w[i] - w[15 - i]).abs() < 1e-6);
        }
    }

    #[test]
    fn hann_peak_at_center() {
        let w = hann_window(9);
        let max = w.iter().cloned().fold(f32::MIN, f32::max);
        assert!((w[4] - max).abs() < 1e-6);
        assert!((w[4] - 1.0).abs() < 1e-6);
    }
}
