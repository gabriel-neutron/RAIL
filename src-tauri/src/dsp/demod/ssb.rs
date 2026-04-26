//! SSB (USB/LSB) demodulation via the phasing method.
//!
//! See `docs/DSP.md` §6 for the sign convention, DC-blocking IIR design,
//! group delay compensation, and filter design.

use num_complex::Complex;

use crate::dsp::filter::{hilbert_fir_taps, sinc_lowpass_taps, ComplexDcBlocker, FirFilter};

/// 129-tap Hilbert FIR. At 16 kHz (SSB_BASEBAND_RATE_HZ) the passband
/// edge is ~125 Hz, covering the full voice band (300–3000 Hz).
/// See `docs/DSP.md` §6.
const HILBERT_TAPS: usize = 129;
/// 65-tap audio LPF per path. Group delay = (65-1)/2 = 32 samples.
/// See `docs/DSP.md` §6 — group delay compensation.
const AUDIO_LPF_TAPS: usize = 65;
/// Audio LPF cutoff for USB/LSB voice channels (Hz). See `docs/DSP.md` §6.
const AUDIO_LPF_CUTOFF_HZ: f32 = 3_000.0;
/// SSB baseband sample rate — must match `demod::SSB_BASEBAND_RATE_HZ`.
const BASEBAND_RATE_HZ: f32 = 16_000.0;
/// DC-blocking HP cutoff: eliminates I/Q bias without affecting voice.
/// See `docs/DSP.md` §6.
const DC_BLOCK_HZ: f32 = 10.0;

/// Streaming SSB demodulator.
///
/// Signal flow per `docs/DSP.md` §6:
/// 1. DC-blocking IIR (10 Hz HP) on complex baseband — removes I/Q offset.
/// 2. 65-tap audio LPF on I and Q independently — 32-sample group delay each.
/// 3. Hilbert FIR on LPF'd Q — 64-sample group delay.
/// 4. I delay buffer (64 samples) — matches Hilbert group delay.
/// 5. Combine: `y = I_delayed ± H{LPF(Q)}` (both paths total 96 samples delay).
///
/// - USB: `y = I_delayed − H{LPF(Q)}`
/// - LSB: `y = I_delayed + H{LPF(Q)}`
pub struct SsbDemodulator {
    dc_blocker: ComplexDcBlocker,
    audio_lpf_i: FirFilter,
    audio_lpf_q: FirFilter,
    hilbert: FirFilter,
    /// Circular delay buffer for LPF'd I — length = `(HILBERT_TAPS − 1) / 2 = 64`.
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
        let hilbert_taps = hilbert_fir_taps(HILBERT_TAPS);
        let audio_taps = sinc_lowpass_taps(AUDIO_LPF_CUTOFF_HZ, BASEBAND_RATE_HZ, AUDIO_LPF_TAPS);
        let hilbert_delay = (HILBERT_TAPS - 1) / 2;
        Self {
            dc_blocker: ComplexDcBlocker::new(DC_BLOCK_HZ, BASEBAND_RATE_HZ),
            audio_lpf_i: FirFilter::new(audio_taps.clone()),
            audio_lpf_q: FirFilter::new(audio_taps),
            hilbert: FirFilter::new(hilbert_taps),
            i_buf: vec![0.0_f32; hilbert_delay],
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
    ///
    /// See `docs/DSP.md` §6 for the full signal flow and delay accounting.
    pub fn process(&mut self, iq: &[Complex<f32>], out: &mut Vec<f32>) {
        out.clear();
        out.reserve(iq.len());
        let delay = self.i_buf.len();
        for &x in iq {
            // 1. DC blocking on complex baseband.
            let x = self.dc_blocker.step(x);
            // 2. Per-path audio LPF (32-sample group delay each).
            let i_lpf = self.audio_lpf_i.step(x.re);
            let q_lpf = self.audio_lpf_q.step(x.im);
            // 3. Hilbert on LPF'd Q (64-sample group delay).
            let q_shifted = self.hilbert.step(q_lpf);
            // 4. Delay LPF'd I by Hilbert group delay (64 samples).
            let i_delayed = if delay == 0 {
                i_lpf
            } else {
                let old = self.i_buf[self.i_head];
                self.i_buf[self.i_head] = i_lpf;
                self.i_head = (self.i_head + 1) % delay;
                old
            };
            // 5. Combine: both paths carry 32 + 64 = 96 samples total delay.
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
    fn dc_offset_is_removed() {
        // Pure DC complex input (I=1, Q=0) — output must converge to zero
        // after the DC-blocking IIR settles (~a few hundred samples at 10 Hz / 16 kHz).
        let dc = vec![Complex::new(1.0_f32, 0.0_f32); 8192];
        let mut usb = SsbDemodulator::new_usb();
        let mut out = Vec::new();
        usb.process(&dc, &mut out);
        let tail_peak = out[out.len() / 2..]
            .iter()
            .fold(0.0_f32, |a, &x| a.max(x.abs()));
        assert!(
            tail_peak < 0.1,
            "DC offset should be suppressed in tail, got peak={tail_peak}"
        );
    }

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

        assert!(
            peak_pass > 1.5,
            "USB should pass positive tone, got {peak_pass}"
        );
        assert!(
            peak_cancel < 0.1,
            "USB should cancel negative tone, got {peak_cancel}"
        );
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

        assert!(
            peak_pass > 1.5,
            "LSB should pass negative tone, got {peak_pass}"
        );
        assert!(
            peak_cancel < 0.1,
            "LSB should cancel positive tone, got {peak_cancel}"
        );
    }
}
