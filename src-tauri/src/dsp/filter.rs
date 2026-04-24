//! Windowing, streaming FIR filters, integer decimation, de-emphasis
//! and linear fractional resampling.
//!
//! Math lives in `docs/DSP.md` §7 — do not re-derive here. These are
//! streaming implementations (persistent delay lines / phase accumulators)
//! so the DSP task can call them once per IQ chunk without per-call
//! allocation.

use std::f32::consts::PI;

use num_complex::Complex;

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

/// Hann-windowed sinc low-pass taps for a cutoff of `cutoff_hz` at
/// `sample_rate_hz`. `n_taps` should be odd so the main lobe lands on
/// the center tap; an even count still works (the ideal impulse is just
/// shifted by a half-sample, no aliasing impact).
///
/// See `docs/DSP.md` §7.
pub fn sinc_lowpass_taps(cutoff_hz: f32, sample_rate_hz: f32, n_taps: usize) -> Vec<f32> {
    assert!(n_taps > 0, "FIR must have at least one tap");
    assert!(
        cutoff_hz > 0.0 && cutoff_hz < 0.5 * sample_rate_hz,
        "cutoff must satisfy 0 < fc < fs/2"
    );

    let m = (n_taps - 1) as f32;
    let window = hann_window(n_taps);
    // Normalized cutoff in cycles/sample. `2·fc/fs` makes the sinc zero
    // crossings fall on ±1/(2·fc/fs) samples from the center.
    let omega = 2.0 * cutoff_hz / sample_rate_hz;

    let mut taps: Vec<f32> = (0..n_taps)
        .map(|i| {
            let x = i as f32 - 0.5 * m;
            let s = if x.abs() < 1e-9 {
                omega
            } else {
                (PI * omega * x).sin() / (PI * x)
            };
            s * window[i]
        })
        .collect();

    // DC-normalize so unity input passes through at unity gain.
    let sum: f32 = taps.iter().sum();
    if sum.abs() > 1e-12 {
        for t in taps.iter_mut() {
            *t /= sum;
        }
    }
    taps
}

/// Streaming real-valued FIR filter with a persistent delay line.
pub struct FirFilter {
    taps: Vec<f32>,
    delay: Vec<f32>,
    head: usize,
}

impl FirFilter {
    pub fn new(taps: Vec<f32>) -> Self {
        let len = taps.len();
        Self {
            taps,
            delay: vec![0.0; len],
            head: 0,
        }
    }

    /// Process one sample, returning one filtered sample.
    pub fn step(&mut self, x: f32) -> f32 {
        let n = self.taps.len();
        self.delay[self.head] = x;

        // Convolve: sum taps[k] · delay[(head - k) mod n].
        let mut acc = 0.0_f32;
        let mut idx = self.head;
        for &t in self.taps.iter() {
            acc += t * self.delay[idx];
            idx = if idx == 0 { n - 1 } else { idx - 1 };
        }
        self.head = (self.head + 1) % n;
        acc
    }
}

/// Integer decimator for real f32 streams: FIR low-pass then keep one
/// out of every `m` samples. Output length is `(in_len + phase) / m`.
pub struct FirDecimatorReal {
    taps: Vec<f32>,
    delay: Vec<f32>,
    head: usize,
    m: usize,
    phase: usize,
}

impl FirDecimatorReal {
    pub fn new(taps: Vec<f32>, m: usize) -> Self {
        assert!(m >= 1, "decimation factor must be >= 1");
        let len = taps.len().max(1);
        Self {
            taps,
            delay: vec![0.0; len],
            head: 0,
            m,
            phase: 0,
        }
    }

    /// Feed `input`, append decimated outputs to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let n = self.taps.len();
        for &x in input {
            self.delay[self.head] = x;
            self.head = (self.head + 1) % n;

            self.phase += 1;
            if self.phase == self.m {
                self.phase = 0;
                let mut acc = 0.0_f32;
                // Most recent sample is at head-1.
                let mut idx = if self.head == 0 { n - 1 } else { self.head - 1 };
                for &t in self.taps.iter() {
                    acc += t * self.delay[idx];
                    idx = if idx == 0 { n - 1 } else { idx - 1 };
                }
                out.push(acc);
            }
        }
    }
}

/// Integer decimator for complex f32 streams — anti-alias FIR then
/// keep one of every `m` complex samples.
pub struct FirDecimatorComplex {
    taps: Vec<f32>,
    delay: Vec<Complex<f32>>,
    head: usize,
    m: usize,
    phase: usize,
}

impl FirDecimatorComplex {
    pub fn new(taps: Vec<f32>, m: usize) -> Self {
        assert!(m >= 1, "decimation factor must be >= 1");
        let len = taps.len().max(1);
        Self {
            taps,
            delay: vec![Complex::new(0.0, 0.0); len],
            head: 0,
            m,
            phase: 0,
        }
    }

    /// Replace taps in place — used when the channel bandwidth changes.
    /// Delay line is preserved to avoid a click on every retune.
    pub fn set_taps(&mut self, taps: Vec<f32>) {
        let new_len = taps.len().max(1);
        if new_len != self.delay.len() {
            self.delay = vec![Complex::new(0.0, 0.0); new_len];
            self.head = 0;
            self.phase = 0;
        }
        self.taps = taps;
    }

    /// Decimation factor (output rate = input rate / m).
    pub fn factor(&self) -> usize {
        self.m
    }

    /// Feed `input`, append decimated outputs to `out`.
    pub fn process(&mut self, input: &[Complex<f32>], out: &mut Vec<Complex<f32>>) {
        let n = self.taps.len();
        for &x in input {
            self.delay[self.head] = x;
            self.head = (self.head + 1) % n;

            self.phase += 1;
            if self.phase == self.m {
                self.phase = 0;
                let mut acc = Complex::new(0.0_f32, 0.0_f32);
                let mut idx = if self.head == 0 { n - 1 } else { self.head - 1 };
                for &t in self.taps.iter() {
                    acc += self.delay[idx] * t;
                    idx = if idx == 0 { n - 1 } else { idx - 1 };
                }
                out.push(acc);
            }
        }
    }
}

/// Hann-windowed Type III FIR Hilbert transformer taps.
///
/// Tap count must be odd. Non-zero only at odd offsets from center:
/// `h[k] = 2/(π·k) · w[k]` for odd k, 0 for even k and center.
/// Group delay = `(n_taps − 1) / 2` samples.
///
/// See `docs/DSP.md` §6.
pub fn hilbert_fir_taps(n_taps: usize) -> Vec<f32> {
    assert!(n_taps >= 3 && n_taps % 2 == 1, "Hilbert FIR needs odd tap count ≥ 3");
    let center = (n_taps - 1) / 2;
    let window = hann_window(n_taps);
    let mut taps = vec![0.0_f32; n_taps];
    for i in 0..n_taps {
        if i == center {
            continue; // center tap is always zero
        }
        let k = i as f32 - center as f32;
        // sin²(π·k/2) = 1 for odd k, 0 for even k — only odd offsets contribute.
        let sin_sq = (PI * k / 2.0).sin().powi(2);
        taps[i] = 2.0 * sin_sq / (PI * k) * window[i];
    }
    taps
}

/// Single-pole IIR de-emphasis filter for broadcast WBFM.
/// `y[n] = α·x[n] + (1−α)·y[n−1]`, with `α = 1 − exp(−1 / (τ·fs))`.
///
/// See `docs/DSP.md` §4 (Europe default: `τ = 50 µs`).
pub struct DeemphasisIir {
    alpha: f32,
    prev: f32,
}

impl DeemphasisIir {
    pub fn new(tau_seconds: f32, sample_rate_hz: f32) -> Self {
        let alpha = 1.0 - (-1.0 / (tau_seconds * sample_rate_hz)).exp();
        Self { alpha, prev: 0.0 }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        let a = self.alpha;
        let one_minus_a = 1.0 - a;
        for s in buf.iter_mut() {
            let y = a * *s + one_minus_a * self.prev;
            self.prev = y;
            *s = y;
        }
    }
}

/// Linear-interpolation fractional resampler. Call with a ratio of
/// `out_rate / in_rate`; `process` produces ≈ `in_len · ratio` samples.
///
/// Input is assumed band-limited below `out_rate/2` by an upstream LPF
/// (see `DemodChain`); linear interp after a proper anti-alias filter
/// is audibly indistinguishable from polyphase resampling at SDR rates.
pub struct LinearResampler {
    step: f32,
    phase: f32,
    prev: f32,
}

impl LinearResampler {
    pub fn new(in_rate_hz: f32, out_rate_hz: f32) -> Self {
        assert!(in_rate_hz > 0.0 && out_rate_hz > 0.0);
        Self {
            step: in_rate_hz / out_rate_hz,
            phase: 0.0,
            prev: 0.0,
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        // `phase` tracks the fractional input index relative to the
        // start of this block. Each output sample advances phase by
        // `step`; when `phase >= i + 1` we cross into the next input
        // sample.
        for (i, &x) in input.iter().enumerate() {
            let i_f = i as f32;
            while self.phase < i_f + 1.0 {
                let frac = self.phase - i_f;
                let y = self.prev * (1.0 - frac) + x * frac;
                out.push(y);
                self.phase += self.step;
            }
            self.prev = x;
        }
        // Re-anchor phase to the next block by subtracting the block
        // length. Keeps the accumulator from growing unbounded.
        self.phase -= input.len() as f32;
    }
}

/// Second-order IIR bandpass filter (RBJ biquad).
///
/// CW usage: center 700 Hz, bandwidth 400 Hz at 16 kHz.
/// See `docs/DSP.md` §6 for the coefficient derivation.
pub struct BiquadBpf {
    b0: f32,
    b2: f32, // b1 = 0 for a pure bandpass
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadBpf {
    /// Create a bandpass centered at `center_hz` with a −3 dB bandwidth
    /// of `bandwidth_hz` at the given `sample_rate_hz`.
    pub fn new(center_hz: f32, bandwidth_hz: f32, sample_rate_hz: f32) -> Self {
        let w0 = 2.0 * PI * center_hz / sample_rate_hz;
        let alpha = w0.sin() / (2.0 * center_hz / bandwidth_hz);
        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            b2: -alpha / a0,
            a1: -2.0 * w0.cos() / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Process one sample.
    pub fn step(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Process a buffer in-place.
    pub fn process_inplace(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            *s = self.step(*s);
        }
    }

    /// Reinitialize with new parameters, resetting state.
    pub fn reconfigure(&mut self, center_hz: f32, bandwidth_hz: f32, sample_rate_hz: f32) {
        *self = Self::new(center_hz, bandwidth_hz, sample_rate_hz);
    }
}

/// Two cascaded `BiquadBpf` stages (4th-order bandpass).
///
/// Used for CW to achieve ~20 dB/octave selectivity — sufficient to suppress
/// adjacent tones from the USB phasing output. See `docs/DSP.md` §6.
pub struct BiquadBpf4 {
    s1: BiquadBpf,
    s2: BiquadBpf,
}

impl BiquadBpf4 {
    /// Create a 4th-order bandpass centered at `center_hz` (Hz) with a
    /// −3 dB bandwidth of `bandwidth_hz` (Hz) at `sample_rate_hz` (Hz).
    pub fn new(center_hz: f32, bandwidth_hz: f32, sample_rate_hz: f32) -> Self {
        Self {
            s1: BiquadBpf::new(center_hz, bandwidth_hz, sample_rate_hz),
            s2: BiquadBpf::new(center_hz, bandwidth_hz, sample_rate_hz),
        }
    }

    /// Process a buffer in-place through both cascaded stages.
    pub fn process_inplace(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            *s = self.s2.step(self.s1.step(*s));
        }
    }

    /// Reinitialize both stages with new parameters, resetting all state.
    pub fn reconfigure(&mut self, center_hz: f32, bandwidth_hz: f32, sample_rate_hz: f32) {
        self.s1.reconfigure(center_hz, bandwidth_hz, sample_rate_hz);
        self.s2.reconfigure(center_hz, bandwidth_hz, sample_rate_hz);
    }
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

    #[test]
    fn sinc_lpf_dc_gain_is_unity() {
        let taps = sinc_lowpass_taps(8_000.0, 48_000.0, 65);
        let sum: f32 = taps.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "DC gain = {sum}");
    }

    #[test]
    fn fir_filter_passes_dc() {
        let taps = sinc_lowpass_taps(5_000.0, 48_000.0, 33);
        let mut f = FirFilter::new(taps);
        // Feed enough samples to fully load the delay line.
        let mut last = 0.0;
        for _ in 0..256 {
            last = f.step(1.0);
        }
        assert!((last - 1.0).abs() < 1e-3, "DC steady state = {last}");
    }

    #[test]
    fn fir_decimator_real_output_length() {
        let taps = sinc_lowpass_taps(5_000.0, 48_000.0, 33);
        let mut d = FirDecimatorReal::new(taps, 4);
        let input = vec![1.0_f32; 1024];
        let mut out = Vec::new();
        d.process(&input, &mut out);
        assert_eq!(out.len(), 1024 / 4);
    }

    #[test]
    fn fir_decimator_complex_passes_dc() {
        let taps = sinc_lowpass_taps(10_000.0, 256_000.0, 65);
        let mut d = FirDecimatorComplex::new(taps, 8);
        let input = vec![Complex::new(1.0_f32, 0.0); 1024];
        let mut out = Vec::new();
        d.process(&input, &mut out);
        assert_eq!(out.len(), 128);
        // Tail samples are past filter load time; should be near 1+0j.
        let tail = out.last().copied().unwrap();
        assert!((tail.re - 1.0).abs() < 1e-3);
        assert!(tail.im.abs() < 1e-3);
    }

    #[test]
    fn deemphasis_settles_toward_dc() {
        let mut f = DeemphasisIir::new(50e-6, 256_000.0);
        let mut buf = vec![1.0_f32; 4096];
        f.process(&mut buf);
        let tail = *buf.last().unwrap();
        assert!((tail - 1.0).abs() < 1e-3);
    }

    #[test]
    fn linear_resampler_output_length_matches_ratio() {
        let mut r = LinearResampler::new(256_000.0, 44_100.0);
        let input = vec![0.5_f32; 25_600];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        let expected = (25_600.0_f32 * 44_100.0 / 256_000.0).round() as usize;
        let diff = out.len() as i64 - expected as i64;
        assert!(diff.abs() <= 1, "got {} expected {}", out.len(), expected);
    }

    #[test]
    fn hilbert_taps_are_antisymmetric_and_center_zero() {
        let n = 65_usize;
        let taps = hilbert_fir_taps(n);
        let center = (n - 1) / 2;
        assert!(taps[center].abs() < 1e-9, "center tap must be zero");
        for i in 0..center {
            assert!(
                (taps[i] + taps[n - 1 - i]).abs() < 1e-6,
                "tap {i} not antisymmetric: {} vs {}",
                taps[i],
                taps[n - 1 - i]
            );
        }
    }

    #[test]
    fn hilbert_taps_even_offsets_are_zero() {
        let taps = hilbert_fir_taps(65);
        let center = 32_usize;
        for i in (0..65).filter(|&i| (i as i32 - center as i32) % 2 == 0 && i != center) {
            assert!(taps[i].abs() < 1e-9, "even-offset tap[{i}] should be 0");
        }
    }

    #[test]
    fn linear_resampler_preserves_dc() {
        let mut r = LinearResampler::new(256_000.0, 44_100.0);
        let input = vec![0.25_f32; 4096];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        for (i, y) in out.iter().enumerate().skip(8) {
            assert!((*y - 0.25).abs() < 1e-4, "sample {i} = {y}");
        }
    }

    #[test]
    fn biquad_bpf4_passes_center_attenuates_stopband() {
        // 4th-order: 700 Hz center, 400 Hz BW, 16 kHz rate (CW parameters from DSP.md §6).
        let fs = 16_000.0_f32;
        let n = 4096_usize;
        let half = n / 2;

        let run = |freq: f32| -> f32 {
            let mut bpf = BiquadBpf4::new(700.0, 400.0, fs);
            let mut buf: Vec<f32> = (0..n)
                .map(|k| (2.0 * PI * freq * k as f32 / fs).cos())
                .collect();
            bpf.process_inplace(&mut buf);
            buf[half..].iter().fold(0.0_f32, |a, &x| a.max(x.abs()))
        };

        let passband_peak = run(700.0);
        let stopband_peak = run(4_000.0);

        assert!(passband_peak > 0.5, "BPF4 passband gain too low: {passband_peak}");
        assert!(stopband_peak < 0.02, "BPF4 stopband not attenuated: {stopband_peak}");
    }
}
