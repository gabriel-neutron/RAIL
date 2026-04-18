//! AM demodulation via envelope detection.
//!
//! See `docs/DSP.md` §5. Output is `|x| − DC`, scaled so full-scale IQ
//! maps to ±1 after the running DC removal settles.

use num_complex::Complex;

/// Streaming AM envelope detector with a 1-pole DC-removal filter.
pub struct AmEnvelope {
    dc: f32,
    /// High-pass pole — smaller = slower DC tracking. Tuned for audio
    /// rates; ~20 Hz corner is roughly `alpha = 20·2π/fs`.
    alpha: f32,
}

impl AmEnvelope {
    /// `sample_rate_hz` is the rate of the complex baseband feeding
    /// `process`. A ~20 Hz high-pass corner keeps voice/music intact.
    pub fn new(sample_rate_hz: f32) -> Self {
        // 1 - exp(-2π·fc / fs), fc = 20 Hz.
        let alpha = 1.0 - (-2.0 * std::f32::consts::PI * 20.0 / sample_rate_hz).exp();
        Self { dc: 0.0, alpha }
    }

    pub fn reconfigure(&mut self, sample_rate_hz: f32) {
        self.alpha = 1.0 - (-2.0 * std::f32::consts::PI * 20.0 / sample_rate_hz).exp();
        self.dc = 0.0;
    }

    /// Process a complex baseband block into a real audio block.
    /// `out` is cleared and overwritten.
    pub fn process(&mut self, iq: &[Complex<f32>], out: &mut Vec<f32>) {
        out.clear();
        out.reserve(iq.len());
        let a = self.alpha;
        let mut dc = self.dc;
        for &x in iq {
            let mag = (x.re * x.re + x.im * x.im).sqrt();
            dc += a * (mag - dc);
            out.push(mag - dc);
        }
        self.dc = dc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_input_converges_to_zero_output() {
        let mut am = AmEnvelope::new(256_000.0);
        let iq = vec![Complex::new(0.7_f32, 0.0); 16_384];
        let mut out = Vec::new();
        am.process(&iq, &mut out);
        let tail = out.last().copied().unwrap();
        assert!(tail.abs() < 1e-3, "tail = {tail}");
    }

    #[test]
    fn amplitude_modulation_survives() {
        // Carrier modulated by a slow |·|: envelope should track it.
        let fs = 64_000.0_f32;
        let mut am = AmEnvelope::new(fs);
        let n = 32_768;
        let mut iq = Vec::with_capacity(n);
        for k in 0..n {
            // 1 kHz modulating tone on top of 0.5 carrier magnitude.
            let t = k as f32 / fs;
            let env = 0.5 + 0.25 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
            iq.push(Complex::new(env, 0.0));
        }
        let mut out = Vec::new();
        am.process(&iq, &mut out);
        let peak = out.iter().skip(4096).fold(0.0_f32, |a, &b| a.max(b.abs()));
        // Expect peak near 0.25 (modulation amplitude), carrier DC
        // removed by the HPF.
        assert!(peak > 0.2 && peak < 0.3, "peak = {peak}");
    }
}
