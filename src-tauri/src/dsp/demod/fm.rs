//! FM demodulation via complex differentiation.
//!
//! See `docs/DSP.md` §4. Instead of two `atan2` calls per sample,
//! compute `arg(x[n] · x[n-1]*)` which is both faster and phase-wrap
//! safe by construction.

use num_complex::Complex;
use std::f32::consts::PI;

/// Streaming FM discriminator. Output is the instantaneous frequency
/// deviation, normalized so full-scale deviation maps to roughly ±1.
pub struct FmDiscriminator {
    prev: Complex<f32>,
    /// `fs / (2π · max_deviation)` — pre-computed normalization gain.
    gain: f32,
}

impl FmDiscriminator {
    /// `sample_rate_hz` is the baseband rate at the discriminator
    /// input; `max_deviation_hz` is the signal's peak deviation
    /// (75 kHz for WBFM, 5 kHz for NBFM).
    pub fn new(sample_rate_hz: f32, max_deviation_hz: f32) -> Self {
        Self {
            prev: Complex::new(0.0, 0.0),
            gain: sample_rate_hz / (2.0 * PI * max_deviation_hz),
        }
    }

    pub fn reconfigure(&mut self, sample_rate_hz: f32, max_deviation_hz: f32) {
        self.gain = sample_rate_hz / (2.0 * PI * max_deviation_hz);
        self.prev = Complex::new(0.0, 0.0);
    }

    /// Process a complex baseband block into a real audio block.
    /// `out` is cleared and overwritten.
    pub fn process(&mut self, iq: &[Complex<f32>], out: &mut Vec<f32>) {
        out.clear();
        out.reserve(iq.len());
        let mut prev = self.prev;
        for &x in iq {
            // arg(x · prev*) == phase difference, wrap-safe.
            let d = x * prev.conj();
            let phase = d.im.atan2(d.re);
            out.push(phase * self.gain);
            prev = x;
        }
        self.prev = prev;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_tone_produces_constant_deviation() {
        // A complex tone at +0.1·fs should demodulate to +0.1·fs/max_dev.
        let fs = 256_000.0_f32;
        let max_dev = 75_000.0_f32;
        let freq = 0.1 * fs; // 25.6 kHz offset
        let n = 4096;
        let mut iq = Vec::with_capacity(n);
        for k in 0..n {
            let t = k as f32 / fs;
            let phase = 2.0 * PI * freq * t;
            iq.push(Complex::new(phase.cos(), phase.sin()));
        }

        let mut disc = FmDiscriminator::new(fs, max_dev);
        let mut out = Vec::new();
        disc.process(&iq, &mut out);

        // Skip the first sample (prev=0 warmup).
        let expected = freq / max_dev;
        for (i, &y) in out.iter().enumerate().skip(1) {
            assert!(
                (y - expected).abs() < 1e-3,
                "sample {i}: got {y}, expected {expected}"
            );
        }
    }

    #[test]
    fn zero_input_yields_zero_output() {
        let mut disc = FmDiscriminator::new(256_000.0, 75_000.0);
        let iq = vec![Complex::new(0.0, 0.0); 128];
        let mut out = Vec::new();
        disc.process(&iq, &mut out);
        assert!(out.iter().all(|s| s.abs() < 1e-6));
    }
}
