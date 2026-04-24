//! SSB (USB/LSB) demodulation via the phasing method.
//!
//! See `docs/DSP.md` §6 for the sign convention and filter design.

use num_complex::Complex;

use crate::dsp::filter::{hilbert_fir_taps, FirFilter};

/// 129-tap Hilbert FIR. At 16 kHz (SSB_BASEBAND_RATE_HZ) the passband
/// edge is ~125 Hz, covering the full voice band (300–3000 Hz).
/// See `docs/DSP.md` §6.
const HILBERT_TAPS: usize = 129;

/// Streaming SSB demodulator.
///
/// Applies a Hilbert FIR to the Q channel, delays the I channel by the
/// same group delay, then combines per `docs/DSP.md` §6:
///
/// - USB: `y = I_delayed − H{Q}`
/// - LSB: `y = I_delayed + H{Q}`
pub struct SsbDemodulator {
    hilbert: FirFilter,
    /// Circular delay buffer for I — length = `(HILBERT_TAPS − 1) / 2`.
    i_buf: Vec<f32>,
    i_head: usize,
    /// `−1.0` for USB, `+1.0` for LSB.
    sign: f32,
}

impl SsbDemodulator {
    /// Build a USB demodulator.
    pub fn new_usb() -> Self {
        Self::new(-1.0)
    }

    /// Build a LSB demodulator.
    pub fn new_lsb() -> Self {
        Self::new(1.0)
    }

    fn new(sign: f32) -> Self {
        let taps = hilbert_fir_taps(HILBERT_TAPS);
        let delay = (HILBERT_TAPS - 1) / 2;
        Self {
            hilbert: FirFilter::new(taps),
            i_buf: vec![0.0_f32; delay],
            i_head: 0,
            sign,
        }
    }

    /// Switch to USB without clearing filter state.
    pub fn set_usb(&mut self) {
        self.sign = -1.0;
    }

    /// Switch to LSB without clearing filter state.
    pub fn set_lsb(&mut self) {
        self.sign = 1.0;
    }

    /// Process one IQ block into real audio. `out` is cleared and overwritten.
    pub fn process(&mut self, iq: &[Complex<f32>], out: &mut Vec<f32>) {
        out.clear();
        out.reserve(iq.len());
        let delay = self.i_buf.len();
        for &x in iq {
            let q_shifted = self.hilbert.step(x.im);
            let i_delayed = if delay == 0 {
                x.re
            } else {
                let old = self.i_buf[self.i_head];
                self.i_buf[self.i_head] = x.re;
                self.i_head = (self.i_head + 1) % delay;
                old
            };
            out.push(i_delayed + self.sign * q_shifted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    // Tests run at the SSB baseband rate (16 kHz) matching DemodChain usage.
    // At this rate a 129-tap Hilbert FIR covers voice down to ~125 Hz.

    #[test]
    fn usb_passes_positive_tone_cancels_negative() {
        // USB: positive-freq tone exp(+jωt) → large output after warmup.
        //      negative-freq tone exp(−jωt) → near-zero (opposite sideband).
        let fs = 16_000.0_f32;
        let fa = 1_000.0_f32; // 1 kHz — well within Hilbert passband at 16 kHz
        let n = 8192_usize;
        let warmup = (HILBERT_TAPS - 1) / 2 + 64;

        let pos: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phi = 2.0 * PI * fa * k as f32 / fs;
                Complex::new(phi.cos(), phi.sin())
            })
            .collect();
        let neg: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phi = 2.0 * PI * fa * k as f32 / fs;
                Complex::new(phi.cos(), -phi.sin())
            })
            .collect();

        let mut usb = SsbDemodulator::new_usb();
        let mut out = Vec::new();
        usb.process(&pos, &mut out);
        let peak_pass = out[warmup..].iter().fold(0.0_f32, |a, &b| a.max(b.abs()));

        let mut usb2 = SsbDemodulator::new_usb();
        usb2.process(&neg, &mut out);
        let peak_cancel = out[warmup..].iter().fold(0.0_f32, |a, &b| a.max(b.abs()));

        assert!(peak_pass > 1.5, "USB should pass positive tone, got {peak_pass}");
        assert!(peak_cancel < 0.1, "USB should cancel negative tone, got {peak_cancel}");
    }

    #[test]
    fn lsb_passes_negative_tone_cancels_positive() {
        let fs = 16_000.0_f32;
        let fa = 1_000.0_f32;
        let n = 8192_usize;
        let warmup = (HILBERT_TAPS - 1) / 2 + 64;

        let pos: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phi = 2.0 * PI * fa * k as f32 / fs;
                Complex::new(phi.cos(), phi.sin())
            })
            .collect();
        let neg: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phi = 2.0 * PI * fa * k as f32 / fs;
                Complex::new(phi.cos(), -phi.sin())
            })
            .collect();

        let mut lsb = SsbDemodulator::new_lsb();
        let mut out = Vec::new();
        lsb.process(&neg, &mut out);
        let peak_pass = out[warmup..].iter().fold(0.0_f32, |a, &b| a.max(b.abs()));

        let mut lsb2 = SsbDemodulator::new_lsb();
        lsb2.process(&pos, &mut out);
        let peak_cancel = out[warmup..].iter().fold(0.0_f32, |a, &b| a.max(b.abs()));

        assert!(peak_pass > 1.5, "LSB should pass negative tone, got {peak_pass}");
        assert!(peak_cancel < 0.1, "LSB should cancel positive tone, got {peak_cancel}");
    }
}
