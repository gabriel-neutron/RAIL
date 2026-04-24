//! Demodulator pipeline: IQ → channel filter → demod → audio filter →
//! de-emphasis → resampler → squelch → f32 PCM.
//!
//! See `docs/DSP.md` §4–5 and `docs/ARCHITECTURE.md` §5. The chain is
//! streaming: all internal filters own persistent delay lines so the
//! DSP task can call `process` once per IQ chunk.

pub mod am;
pub mod fm;
pub mod ssb;

use num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::dsp::demod::am::AmEnvelope;
use crate::dsp::demod::fm::FmDiscriminator;
use crate::dsp::demod::ssb::SsbDemodulator;
use crate::dsp::filter::{
    sinc_lowpass_taps, BiquadBpf4, DeemphasisIir, FirDecimatorComplex, FirFilter, LinearResampler,
};

/// Baseband rate for FM/NFM/AM: 2.048 MHz / 8 = 256 kHz.
pub const BASEBAND_RATE_HZ: f32 = 256_000.0;
/// Baseband rate for USB/LSB: 2.048 MHz / 128 = 16 kHz.
/// At this rate a 129-tap Hilbert FIR covers voice down to ~125 Hz.
/// See `docs/DSP.md` §6.
pub const SSB_BASEBAND_RATE_HZ: f32 = 16_000.0;

/// Audio output sample rate delivered to the frontend.
pub const AUDIO_RATE_HZ: f32 = 44_100.0;

const CHANNEL_FIR_TAPS: usize = 65;
const AUDIO_FIR_TAPS: usize = 65;
/// 50 µs de-emphasis time constant (Europe, `docs/DSP.md` §4).
const DEEMPHASIS_TAU_S: f32 = 50e-6;
/// CW sidetone BPF center frequency — standard 700 Hz offset (docs/DSP.md §6).
const CW_BPF_CENTER_HZ: f32 = 700.0;
/// CW BPF −3 dB bandwidth (docs/DSP.md §6).
const CW_BPF_BW_HZ: f32 = 400.0;

/// User-facing demodulator modes.
///
/// `Fm` = WBFM broadcast (200 kHz); `Nfm` = narrowband FM voice (12.5 kHz);
/// `Am` = envelope detection; `Usb`/`Lsb` = phasing SSB (3 kHz voice);
/// `Cw` = USB phasing + 700 Hz BPF for Morse.
/// See `docs/DSP.md` §4–6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DemodMode {
    Fm,
    Nfm,
    Am,
    Usb,
    Lsb,
    Cw,
}

/// Runtime control messages from Tauri commands to the DSP task.
#[derive(Debug, Clone, Copy)]
pub enum DemodControl {
    SetMode(DemodMode),
    SetBandwidthHz(f32),
    /// Threshold in dBFS; `f32::NEG_INFINITY` disables squelch.
    SetSquelchDbfs(f32),
}

/// Current chain configuration. Kept as plain data so the DSP task can
/// apply changes incrementally.
#[derive(Debug, Clone, Copy)]
pub struct DemodConfig {
    pub mode: DemodMode,
    pub bandwidth_hz: f32,
    pub squelch_dbfs: f32,
}

impl Default for DemodConfig {
    fn default() -> Self {
        Self {
            mode: DemodMode::Fm,
            bandwidth_hz: 200_000.0,
            squelch_dbfs: f32::NEG_INFINITY,
        }
    }
}

/// Full demodulator chain. Owns every filter, demodulator and buffer —
/// the DSP task just feeds IQ chunks in and reads f32 PCM out.
pub struct DemodChain {
    input_rate_hz: f32,
    baseband_rate_hz: f32,
    audio_rate_hz: f32,
    decim: FirDecimatorComplex,
    fm: FmDiscriminator,
    am: AmEnvelope,
    ssb: SsbDemodulator,
    audio_lpf: FirFilter,
    /// 4th-order IIR bandpass for CW (700 Hz ± 200 Hz). See `docs/DSP.md` §6.
    cw_bpf: BiquadBpf4,
    deemph: DeemphasisIir,
    resampler: LinearResampler,
    config: DemodConfig,
    /// `true` when the current mode is WBFM (broadcast) — gates the
    /// de-emphasis filter.
    wbfm: bool,
    // Scratch buffers, reused between calls to avoid allocations.
    baseband: Vec<Complex<f32>>,
    raw_audio: Vec<f32>,
    filtered_audio: Vec<f32>,
}

impl DemodChain {
    /// Build a chain for the given IQ input rate. Default config is
    /// WBFM with 200 kHz bandwidth and squelch disabled.
    pub fn new(input_rate_hz: f32) -> Self {
        let config = DemodConfig::default();
        Self::with_config(input_rate_hz, config)
    }

    /// Build a chain for the given IQ input rate with an explicit starting config.
    pub fn with_config(input_rate_hz: f32, config: DemodConfig) -> Self {
        let bb_rate = baseband_rate_for(config.mode);
        assert!(input_rate_hz > bb_rate);
        let decim_factor = (input_rate_hz / bb_rate).round() as usize;
        let channel_cutoff = channel_cutoff_for(config.bandwidth_hz, bb_rate);
        let chan_taps = sinc_lowpass_taps(channel_cutoff, input_rate_hz, CHANNEL_FIR_TAPS);

        let (audio_cutoff, max_dev_hz, wbfm) = mode_params(config.mode, config.bandwidth_hz);
        let audio_taps = sinc_lowpass_taps(audio_cutoff, bb_rate, AUDIO_FIR_TAPS);
        // Guard FM discriminator against zero deviation (USB/LSB don't use it).
        let fm_dev = if max_dev_hz > 0.0 { max_dev_hz } else { 5_000.0 };

        Self {
            input_rate_hz,
            baseband_rate_hz: bb_rate,
            audio_rate_hz: AUDIO_RATE_HZ,
            decim: FirDecimatorComplex::new(chan_taps, decim_factor),
            fm: FmDiscriminator::new(bb_rate, fm_dev),
            am: AmEnvelope::new(bb_rate),
            ssb: ssb_for_mode(config.mode),
            audio_lpf: FirFilter::new(audio_taps),
            cw_bpf: BiquadBpf4::new(CW_BPF_CENTER_HZ, CW_BPF_BW_HZ, bb_rate),
            deemph: DeemphasisIir::new(DEEMPHASIS_TAU_S, bb_rate),
            resampler: LinearResampler::new(bb_rate, AUDIO_RATE_HZ),
            config,
            wbfm,
            baseband: Vec::with_capacity(4096),
            raw_audio: Vec::with_capacity(4096),
            filtered_audio: Vec::with_capacity(4096),
        }
    }

    /// Nominal audio sample rate (Hz).
    pub fn audio_rate_hz(&self) -> f32 {
        self.audio_rate_hz
    }

    /// Apply a runtime control message. No-op if the message doesn't
    /// change state.
    pub fn apply(&mut self, msg: DemodControl) {
        match msg {
            DemodControl::SetMode(mode) => {
                if self.config.mode != mode {
                    self.config.mode = mode;
                    self.reconfigure_mode();
                }
            }
            DemodControl::SetBandwidthHz(bw) => {
                if (self.config.bandwidth_hz - bw).abs() > 0.5 {
                    self.config.bandwidth_hz = bw;
                    self.reconfigure_channel();
                    // Changing bandwidth also flips wbfm/deviation for
                    // FM (e.g. 200 kHz → 15 kHz narrows to NBFM).
                    self.reconfigure_mode();
                }
            }
            DemodControl::SetSquelchDbfs(db) => {
                self.config.squelch_dbfs = db;
            }
        }
    }

    fn reconfigure_channel(&mut self) {
        let cutoff = channel_cutoff_for(self.config.bandwidth_hz, self.baseband_rate_hz);
        let taps = sinc_lowpass_taps(cutoff, self.input_rate_hz, CHANNEL_FIR_TAPS);
        let factor = (self.input_rate_hz / self.baseband_rate_hz).round() as usize;
        if self.decim.factor() == factor {
            self.decim.set_taps(taps);
        } else {
            self.decim = FirDecimatorComplex::new(taps, factor);
        }
    }

    fn reconfigure_mode(&mut self) {
        // Switch baseband rate when crossing between FM/AM and SSB families.
        let new_bb_rate = baseband_rate_for(self.config.mode);
        if (new_bb_rate - self.baseband_rate_hz).abs() > 0.1 {
            self.baseband_rate_hz = new_bb_rate;
            let factor = (self.input_rate_hz / new_bb_rate).round() as usize;
            let cutoff = channel_cutoff_for(self.config.bandwidth_hz, new_bb_rate);
            let taps = sinc_lowpass_taps(cutoff, self.input_rate_hz, CHANNEL_FIR_TAPS);
            self.decim = FirDecimatorComplex::new(taps, factor);
            self.resampler = LinearResampler::new(new_bb_rate, self.audio_rate_hz);
        }

        let (audio_cutoff, max_dev_hz, wbfm) =
            mode_params(self.config.mode, self.config.bandwidth_hz);
        let audio_taps = sinc_lowpass_taps(audio_cutoff, self.baseband_rate_hz, AUDIO_FIR_TAPS);
        self.audio_lpf = FirFilter::new(audio_taps);
        if matches!(self.config.mode, DemodMode::Fm | DemodMode::Nfm) {
            self.fm.reconfigure(self.baseband_rate_hz, max_dev_hz);
        }
        self.am.reconfigure(self.baseband_rate_hz);
        match self.config.mode {
            DemodMode::Usb | DemodMode::Cw => self.ssb.set_usb(),
            DemodMode::Lsb => self.ssb.set_lsb(),
            _ => {}
        }
        if self.config.mode == DemodMode::Cw {
            self.cw_bpf
                .reconfigure(CW_BPF_CENTER_HZ, CW_BPF_BW_HZ, self.baseband_rate_hz);
        }
        self.deemph = DeemphasisIir::new(DEEMPHASIS_TAU_S, self.baseband_rate_hz);
        self.wbfm = wbfm;
    }

    /// Feed one IQ chunk (pre-fs/4-shifted complex baseband at
    /// `input_rate_hz`) and append f32 PCM to `audio`. Returns the
    /// post-channel-filter RMS power in dBFS (for UI squelch display).
    pub fn process(&mut self, iq: &[Complex<f32>], audio: &mut Vec<f32>) -> f32 {
        self.baseband.clear();
        self.decim.process(iq, &mut self.baseband);

        let rms_dbfs = complex_rms_dbfs(&self.baseband);
        let gated = self.config.squelch_dbfs.is_finite() && rms_dbfs < self.config.squelch_dbfs;

        match self.config.mode {
            DemodMode::Fm | DemodMode::Nfm => {
                self.fm.process(&self.baseband, &mut self.raw_audio)
            }
            DemodMode::Am => self.am.process(&self.baseband, &mut self.raw_audio),
            DemodMode::Usb | DemodMode::Lsb | DemodMode::Cw => {
                self.ssb.process(&self.baseband, &mut self.raw_audio)
            }
        }

        // Audio LPF is the anti-alias for the resampler.
        self.filtered_audio.clear();
        self.filtered_audio.reserve(self.raw_audio.len());
        for &x in self.raw_audio.iter() {
            self.filtered_audio.push(self.audio_lpf.step(x));
        }

        // WBFM de-emphasis only — `self.wbfm` is false for NFM/AM/SSB/CW.
        if self.wbfm {
            self.deemph.process(&mut self.filtered_audio);
        }

        // CW: narrow BPF selects the 700 Hz sidetone (docs/DSP.md §6).
        if self.config.mode == DemodMode::Cw {
            self.cw_bpf.process_inplace(&mut self.filtered_audio);
        }

        let mark = audio.len();
        self.resampler.process(&self.filtered_audio, audio);

        if gated {
            for s in audio[mark..].iter_mut() {
                *s = 0.0;
            }
        }

        rms_dbfs
    }
}

/// Channel-filter cutoff for a user-facing bandwidth, bounded by the
/// Nyquist of the target baseband rate (90 % of `baseband_rate_hz / 2`).
fn channel_cutoff_for(bandwidth_hz: f32, baseband_rate_hz: f32) -> f32 {
    let half = 0.5 * bandwidth_hz;
    let ceiling = 0.9 * 0.5 * baseband_rate_hz;
    half.clamp(1_000.0, ceiling)
}

/// Pick the baseband rate for a given mode.
/// USB/LSB use 16 kHz so the Hilbert FIR covers the voice band; all
/// other modes use 256 kHz for WBFM bandwidth. See `docs/DSP.md` §6.
fn baseband_rate_for(mode: DemodMode) -> f32 {
    match mode {
        DemodMode::Usb | DemodMode::Lsb | DemodMode::Cw => SSB_BASEBAND_RATE_HZ,
        _ => BASEBAND_RATE_HZ,
    }
}

/// Derive audio-LPF cutoff, FM max-deviation and WBFM flag from mode + bandwidth.
///
/// - WBFM (`Fm`, bandwidth ≥ 100 kHz): 75 kHz deviation, 15 kHz LPF, de-emphasis on.
/// - `Fm` narrow (< 100 kHz): NBFM — 5 kHz deviation, 3 kHz LPF.
/// - `Nfm`: always NBFM — 5 kHz deviation, 3 kHz LPF, no de-emphasis.
/// - `Am`: 5 kHz audio LPF (voice-grade).
/// - `Usb`/`Lsb`: 3 kHz audio LPF; max_dev unused (0.0 sentinel).
/// - `Cw`: 2 kHz LPF (anti-alias only); `BiquadBpf4` handles tone selection.
fn mode_params(mode: DemodMode, bandwidth_hz: f32) -> (f32, f32, bool) {
    match mode {
        DemodMode::Fm => {
            if bandwidth_hz >= 100_000.0 {
                (15_000.0, 75_000.0, true)
            } else {
                (3_000.0, 5_000.0, false)
            }
        }
        DemodMode::Nfm => (3_000.0, 5_000.0, false),
        DemodMode::Am => (5_000.0, 0.0, false),
        DemodMode::Usb | DemodMode::Lsb => (3_000.0, 0.0, false),
        // CW: 2 kHz LPF for anti-alias; narrow BPF at 700 Hz applied separately.
        DemodMode::Cw => (2_000.0, 0.0, false),
    }
}

/// Construct the right `SsbDemodulator` variant for `mode`.
/// CW uses USB phasing; non-SSB modes default to USB (not called in hot path).
fn ssb_for_mode(mode: DemodMode) -> SsbDemodulator {
    match mode {
        DemodMode::Lsb => SsbDemodulator::new_lsb(),
        _ => SsbDemodulator::new_usb(),
    }
}

fn complex_rms_dbfs(samples: &[Complex<f32>]) -> f32 {
    if samples.is_empty() {
        return f32::NEG_INFINITY;
    }
    let sum: f32 = samples.iter().map(|c| c.re * c.re + c.im * c.im).sum();
    let mean = sum / samples.len() as f32;
    if mean <= 0.0 {
        f32::NEG_INFINITY
    } else {
        10.0 * mean.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn chain_fm_demodulates_tone() {
        // Synthesize an FM signal at IQ rate: constant deviation of
        // +37.5 kHz (= 0.5·max_dev_wbfm) should produce steady ~0.5.
        let fs = 2_048_000.0_f32;
        let dev = 37_500.0_f32;
        let n = 16_384; // ~8 ms
        let mut iq = Vec::with_capacity(n);
        let mut phase = 0.0_f32;
        let step = 2.0 * PI * dev / fs;
        for _ in 0..n {
            phase += step;
            iq.push(Complex::new(phase.cos(), phase.sin()));
        }

        let mut chain = DemodChain::new(fs);
        let mut audio = Vec::new();
        let rms = chain.process(&iq, &mut audio);
        assert!(rms > -1.0, "unit-amplitude tone RMS should be near 0 dBFS");
        // After filter transients settle, the steady-state should
        // sit near 0.5 (pre-deemphasis). De-emphasis attenuates DC
        // only slightly for a steady deviation (tau·fs >> 1).
        let tail =
            audio[audio.len() / 2..].iter().sum::<f32>() / (audio.len() - audio.len() / 2) as f32;
        assert!(
            (tail - 0.5).abs() < 0.1,
            "expected ~0.5 steady-state, got {tail}"
        );
    }

    #[test]
    fn chain_squelch_silences_audio() {
        let fs = 2_048_000.0_f32;
        let mut chain = DemodChain::with_config(
            fs,
            DemodConfig {
                mode: DemodMode::Fm,
                bandwidth_hz: 200_000.0,
                squelch_dbfs: -10.0,
            },
        );
        // Very weak signal: unit-amplitude noise scaled down hard so
        // the post-filter RMS lands below -10 dBFS.
        let iq = vec![Complex::new(0.001, 0.001); 8_192];
        let mut audio = Vec::new();
        chain.process(&iq, &mut audio);
        assert!(audio.iter().all(|&s| s.abs() < 1e-9));
    }

    #[test]
    fn chain_am_demodulates_amplitude() {
        // AM carrier with a slow amplitude ramp: envelope should
        // track (after HPF removes the DC carrier).
        let fs = 2_048_000.0_f32;
        let n = 16_384;
        let mut iq = Vec::with_capacity(n);
        for k in 0..n {
            let env = 0.4 + 0.2 * (2.0 * PI * 500.0 * k as f32 / fs).sin();
            iq.push(Complex::new(env, 0.0));
        }
        let mut chain = DemodChain::with_config(
            fs,
            DemodConfig {
                mode: DemodMode::Am,
                bandwidth_hz: 10_000.0,
                squelch_dbfs: f32::NEG_INFINITY,
            },
        );
        let mut audio = Vec::new();
        chain.process(&iq, &mut audio);
        // Expect non-zero audio after warmup.
        let tail_peak = audio
            .iter()
            .skip(audio.len() / 2)
            .fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(tail_peak > 0.05, "AM envelope silent: peak = {tail_peak}");
    }

    #[test]
    fn chain_nfm_demodulates_tone() {
        // NFM: constant +2.5 kHz deviation (= 0.5·max_dev_nfm) should
        // produce steady ~0.5, same math as WBFM but different max_dev.
        let fs = 2_048_000.0_f32;
        let dev = 2_500.0_f32;
        let n = 16_384;
        let mut iq = Vec::with_capacity(n);
        let mut phase = 0.0_f32;
        let step = 2.0 * PI * dev / fs;
        for _ in 0..n {
            phase += step;
            iq.push(Complex::new(phase.cos(), phase.sin()));
        }
        let mut chain = DemodChain::with_config(
            fs,
            DemodConfig {
                mode: DemodMode::Nfm,
                bandwidth_hz: 12_500.0,
                squelch_dbfs: f32::NEG_INFINITY,
            },
        );
        let mut audio = Vec::new();
        chain.process(&iq, &mut audio);
        let tail =
            audio[audio.len() / 2..].iter().sum::<f32>() / (audio.len() - audio.len() / 2) as f32;
        assert!(
            (tail - 0.5).abs() < 0.15,
            "NFM: expected ~0.5 steady-state, got {tail}"
        );
    }

    #[test]
    fn chain_usb_produces_audio_from_positive_tone() {
        // USB: positive-freq IQ tone (exp(+jωt)) should survive the chain.
        let fs = 2_048_000.0_f32;
        let fa = 500.0_f32; // well within 3 kHz USB channel
        let n = 16_384_usize;
        let iq: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phi = 2.0 * PI * fa * k as f32 / fs;
                Complex::new(phi.cos(), phi.sin())
            })
            .collect();
        let mut chain = DemodChain::with_config(
            fs,
            DemodConfig {
                mode: DemodMode::Usb,
                bandwidth_hz: 3_000.0,
                squelch_dbfs: f32::NEG_INFINITY,
            },
        );
        let mut audio = Vec::new();
        chain.process(&iq, &mut audio);
        let peak = audio
            .iter()
            .skip(audio.len() / 2)
            .fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(peak > 0.05, "USB chain: expected audio from positive tone, got peak={peak}");
    }

    #[test]
    fn chain_lsb_produces_audio_from_negative_tone() {
        // LSB: negative-freq IQ tone (exp(−jωt)) should survive the chain.
        let fs = 2_048_000.0_f32;
        let fa = 500.0_f32;
        let n = 16_384_usize;
        let iq: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phi = 2.0 * PI * fa * k as f32 / fs;
                Complex::new(phi.cos(), -phi.sin()) // negative sideband
            })
            .collect();
        let mut chain = DemodChain::with_config(
            fs,
            DemodConfig {
                mode: DemodMode::Lsb,
                bandwidth_hz: 3_000.0,
                squelch_dbfs: f32::NEG_INFINITY,
            },
        );
        let mut audio = Vec::new();
        chain.process(&iq, &mut audio);
        let peak = audio
            .iter()
            .skip(audio.len() / 2)
            .fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(peak > 0.05, "LSB chain: expected audio from negative tone, got peak={peak}");
    }

    #[test]
    fn chain_cw_produces_audio_from_700hz_tone() {
        // CW: a 700 Hz positive-freq IQ tone should survive as audible output.
        // The BPF is centered at 700 Hz so this tone lands in its passband.
        let fs = 2_048_000.0_f32;
        let fa = 700.0_f32;
        let n = 16_384_usize;
        let iq: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phi = 2.0 * PI * fa * k as f32 / fs;
                Complex::new(phi.cos(), phi.sin())
            })
            .collect();
        let mut chain = DemodChain::with_config(
            fs,
            DemodConfig {
                mode: DemodMode::Cw,
                bandwidth_hz: 500.0,
                squelch_dbfs: f32::NEG_INFINITY,
            },
        );
        let mut audio = Vec::new();
        chain.process(&iq, &mut audio);
        let peak = audio
            .iter()
            .skip(audio.len() / 2)
            .fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(peak > 0.01, "CW chain: expected audio from 700 Hz tone, got peak={peak}");
    }

    #[test]
    fn chain_cw_attenuates_out_of_band_tone() {
        // CW: a 4 kHz tone should be heavily attenuated by the 700 Hz BPF.
        let fs = 2_048_000.0_f32;
        let fa = 4_000.0_f32;
        let n = 16_384_usize;
        let iq: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phi = 2.0 * PI * fa * k as f32 / fs;
                Complex::new(phi.cos(), phi.sin())
            })
            .collect();
        let mut chain = DemodChain::with_config(
            fs,
            DemodConfig {
                mode: DemodMode::Cw,
                bandwidth_hz: 500.0,
                squelch_dbfs: f32::NEG_INFINITY,
            },
        );
        let mut audio = Vec::new();
        chain.process(&iq, &mut audio);
        let peak = audio
            .iter()
            .skip(audio.len() / 2)
            .fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(peak < 0.05, "CW chain: out-of-band tone should be silent, got peak={peak}");
    }

    #[test]
    fn chain_rms_dbfs_is_monotonic_in_amplitude() {
        // Returned `rms_dbfs` feeds the signal meter. Doubling the IQ
        // amplitude must increase the reading by ~6 dB.
        let fs = 2_048_000.0_f32;
        let make_chain = || DemodChain::new(fs);
        let n = 8_192;

        let weak: Vec<Complex<f32>> = (0..n)
            .map(|k| {
                let phase = 2.0 * PI * 10_000.0 * k as f32 / fs;
                Complex::new(0.1 * phase.cos(), 0.1 * phase.sin())
            })
            .collect();
        let strong: Vec<Complex<f32>> = weak.iter().map(|c| c * 2.0).collect();

        let mut chain_weak = make_chain();
        let mut chain_strong = make_chain();
        let mut drain = Vec::new();
        let rms_weak = chain_weak.process(&weak, &mut drain);
        drain.clear();
        let rms_strong = chain_strong.process(&strong, &mut drain);

        assert!(rms_weak.is_finite() && rms_strong.is_finite());
        let delta = rms_strong - rms_weak;
        assert!(
            (delta - 6.0).abs() < 0.5,
            "expected ~6 dB jump, got {delta} (weak={rms_weak}, strong={rms_strong})"
        );
    }
}
