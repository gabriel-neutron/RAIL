# DSP.md — Digital Signal Processing Reference

## Table of contents
1. [Sampling theory fundamentals](#1-sampling-theory-fundamentals)
2. [FFT and spectral analysis](#2-fft-and-spectral-analysis)
3. [Waterfall pipeline](#3-waterfall-pipeline)
4. [FM demodulation](#4-fm-demodulation)
5. [AM demodulation](#5-am-demodulation)
6. [USB/LSB/CW demodulation](#6-usblsbcw-demodulation)
7. [Filter design](#7-filter-design)
8. [Edge cases and known pitfalls](#8-edge-cases-and-known-pitfalls)

---

## 1. Sampling theory fundamentals

The RTL-SDR outputs **complex IQ samples** (in-phase + quadrature).
Each sample is a pair `(I, Q)` representing a point on the complex plane:

```
x(t) = I(t) + j·Q(t)
```

**Nyquist theorem**: to represent a signal of bandwidth B, the sample rate
must be at least `fs ≥ 2B`. The RTL-SDR captures a complex baseband signal,
so the usable bandwidth equals `fs` (not `fs/2` as in real-valued ADCs).

**Typical RTL-SDR sample rates**: 225 kHz–3.2 MHz (stable: 2.048 MHz, 2.4 MHz).
Rates above 3.2 MHz cause dropped samples on most hardware.

**DC spike**: RTL-SDR produces a DC offset artifact at center frequency.
Center frequency should be offset by ~fs/4 from the signal of interest,
then digitally retuned in software.

---

## 2. FFT and spectral analysis

Library used: `rustfft`. Do not reimplement FFT.

**FFT size (N)**: tradeoff between frequency resolution and time resolution.
- Frequency resolution: `Δf = fs / N`
- Time per frame: `T = N / fs`
- Recommended default: N = 2048 (balanced for waterfall display)
- Valid sizes: powers of 2 for efficiency (`rustfft` accepts any, but 2^n is faster)

**Process per frame**:
1. Read N complex samples from the IQ buffer
2. Apply window function (see §7) to reduce spectral leakage
3. Compute FFT → complex output of length N
4. Compute magnitude: `|X[k]| = sqrt(Re²+ Im²)`
5. Convert to dB: `P[k] = 20·log10(|X[k]|)` (or `10·log10(|X[k]|²)`)
6. Apply FFT shift: move DC bin from index 0 to center (swap halves)
7. Send float32 array of length N to frontend via binary Tauri event

**Normalization**: divide magnitude by N before dB conversion to get
consistent power readings across different FFT sizes.

---

## 3. Waterfall pipeline

```
RTL-SDR USB → IQ buffer (Rust) → FFT → magnitude bins (float32[N])
→ Tauri binary event → React canvas → colormap → pixel row
→ scroll waterfall downward each frame
```

**Frontend responsibility**: colormap only (float32 dB value → RGB color).
Rust must never send RGB. React must never compute FFT or magnitude.

**Recommended colormap**: linear interpolation across:
`[dark blue → blue → cyan → green → yellow → red]`
mapped to the dB range `[noise_floor, signal_peak]`.

**Frame rate**: target 25–30 fps. At fs=2.048 MHz and N=2048:
`T = 2048/2048000 ≈ 1ms per FFT`. Average or skip frames to hit target fps.

---

## 4. FM demodulation

### Wideband FM (WBFM — broadcast, 200 kHz bandwidth)

FM encodes information in instantaneous frequency deviation:
```
s(t) = A·cos(2π·fc·t + 2π·kf·∫m(τ)dτ)
```
where `m(t)` is the audio signal and `kf` is the frequency sensitivity.

**Demodulation via complex differentiation (owned implementation)**:

Given complex IQ samples `x[n] = I[n] + j·Q[n]`:

```
φ[n] = arg(x[n]) = atan2(Q[n], I[n])
m[n] = φ[n] - φ[n-1]   (phase difference = instantaneous frequency)
```

Wrap `m[n]` to `[-π, π]` to handle phase discontinuities.
Scale by `fs / (2π·max_deviation)` to normalize audio amplitude.

**Max deviation**: WBFM = 75 kHz, NBFM = 2.5–5 kHz.

**De-emphasis filter (WBFM only)**: broadcast FM applies 50µs (Europe) or
75µs (USA) pre-emphasis. Apply inverse RC filter post-demodulation:
```
H(z) = (1 - e^(-1/τfs)) / (1 - e^(-1/τfs)·z^(-1))
where τ = 50×10⁻⁶ (Europe)
```

**Decimation**: WBFM — decimate from fs to ~200 kHz before demodulation,
then to ~44.1 kHz for audio output. Use integer decimation ratios where possible.

### Narrowband FM (NBFM — PMR, aviation voice)
Same algorithm, different deviation (2.5–5 kHz) and narrower channel filter.
Explicit `DemodMode::Nfm` in Rust — 5 kHz max deviation, 12.5 kHz channel BW,
3 kHz audio LPF. No 50/75 µs de-emphasis (voice shelf instead).

---

## 5. AM demodulation

AM encodes information in amplitude:
```
s(t) = [A + m(t)]·cos(2π·fc·t)
```

**Demodulation (envelope detection)**:
```
m[n] = |x[n]| = sqrt(I[n]² + Q[n]²)
```

Remove DC: subtract running mean to eliminate carrier offset.
Normalize to audio range `[-1, 1]`.

This is the simplest demodulator — no phase tracking needed.

---

## 6. USB/LSB/CW demodulation

SSB demodulation uses the **phasing method** on the complex IQ signal.

**Sign convention** (RTL-SDR outputs I+jQ, downconverted with exp(−jωc·t)):

For a USB tone at +fa: IQ baseband = exp(+j·2π·fa·t), so I=cos, Q=sin.
For a LSB tone at −fa: IQ baseband = exp(−j·2π·fa·t), so I=cos, Q=−sin.

With Hilbert defined as H{cos(ω·t)} = sin(ω·t), H{sin(ω·t)} = −cos(ω·t):

```
USB: y[n] = I_delayed[n] − H{Q[n]}
LSB: y[n] = I_delayed[n] + H{Q[n]}
```

`I_delayed` is I delayed by `(hilbert_taps − 1) / 2` samples to align with
the Hilbert FIR group delay.

**Hilbert FIR taps** (Type III, antisymmetric, odd tap count N):
```
h[k] = 0                            if k == center
h[k] = 2·sin²(π·k/2) / (π·k) · w[k]  otherwise
     = 2/(π·k) · w[k]  for odd k (sin² = 1)
     = 0                for even k (sin² = 0)
```
where k = tap index − center, w[k] = Hann window.

**Decimation**: channel filter bandwidth set to 3 kHz. After Hilbert combine,
apply audio LPF at 3 kHz, then resample to 44.1 kHz.

### CW (Continuous Wave / Morse)

CW uses the same phasing method as USB (carrier above zero offset), followed
by a narrow IIR bandpass filter (BPF) that selects the CW sidetone frequency.

**BPF design** (second-order RBJ biquad, §7):
- Center: 700 Hz (standard CW sidetone, user tunes ~700 Hz above/below carrier)
- Bandwidth: 400 Hz (−3 dB points at ~500 Hz and ~900 Hz)
- Sample rate: 16 kHz (same `SSB_BASEBAND_RATE_HZ` as USB/LSB)

```
w0    = 2π · 700 / 16000
Q     = 700 / 400 = 1.75
alpha = sin(w0) / (2Q)
b0    =  alpha / (1 + alpha),  b1 = 0,  b2 = −b0
a1    = −2·cos(w0) / (1 + alpha)
a2    = (1 − alpha) / (1 + alpha)
y[n]  = b0·x[n] + b2·x[n−2] − a1·y[n−1] − a2·y[n−2]
```

**Pipeline**: USB SSB demod → 2 kHz LPF (anti-alias) → 700 Hz BPF → resample to 44.1 kHz.

**Squelch note — NFM vs WBFM**: NFM's channel bandwidth (12.5 kHz) is ~16× narrower
than WBFM (200 kHz), so integrated noise power is ~12 dB lower:
`Δ = 10·log10(12500 / 200000) ≈ −12 dB`.
When switching between WBFM and NFM, the UI scales the active squelch threshold
by this offset so the gate position (in SNR terms) stays constant.

---

## 7. Filter design

### Window functions (for FFT spectral analysis)

Applied to the time-domain samples before FFT to reduce spectral leakage.

| Window | Sidelobe level | Main lobe width | Use case |
|---|---|---|---|
| Rectangular | High (-13 dB) | Narrow | Never — aliasing artifacts |
| Hann | Medium (-31 dB) | Medium | Default for waterfall |
| Blackman-Harris | Low (-92 dB) | Wide | Weak signal detection |

**Default**: Hann window. Formula for N-point window:
```
w[n] = 0.5·(1 - cos(2π·n / (N-1)))   for n = 0..N-1
```

### Low-pass FIR filter (for decimation)

Use a windowed-sinc filter before decimating to prevent aliasing.
Cutoff frequency: `fc = fs_output / 2`.

```
h[n] = sinc(2·fc·(n - M/2)) · w[n]
```

where M is filter order (higher = sharper rolloff, more CPU).
Recommended: M = 64 for decimation chains.

Library option: `biquad` crate for IIR filters (simpler, lower CPU).

---

## 8. Edge cases and known pitfalls

| Issue | Cause | Mitigation |
|---|---|---|
| DC spike in waterfall | RTL-SDR LO leakage | Offset center freq by fs/4, retune digitally |
| Phase wrap in FM demod | `atan2` discontinuity at ±π | Wrap difference to `[-π, π]` |
| Audio clicking | Buffer underrun | Ring buffer with ≥3 frames headroom |
| Dropped IQ samples | USB bandwidth exceeded | Cap sample rate at 2.4 MHz max |
| Gain overload (clipping) | AGC off, strong signal | Expose manual gain control in UI |
| FFT size mismatch | N not matching buffer | Assert N == buffer size before FFT |
| Normalization drift | No reference level | Fix noise floor reference at startup |
