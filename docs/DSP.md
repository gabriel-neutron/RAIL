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
9. [Display coordinate systems (bin ↔ Hz ↔ pixel)](#9-display-coordinate-systems-bin--hz--pixel)

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
- Current value: N = 8192 (250 Hz/bin at 2.048 MHz sample rate)
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

**Colormap**: linear interpolation across a seven-stop single-hue amber ramp,
mapped to the dB range `[noise_floor, signal_peak]`:

| # | RGB | Role |
|---|---|---|
| 0 | `4, 3, 1` | cold tube — below the noise floor |
| 1 | `46, 25, 4` | |
| 2 | `104, 59, 9` | |
| 3 | `168, 105, 18` | |
| 4 | `224, 152, 31` | |
| 5 | `255, 196, 84` | |
| 6 | `255, 246, 226` | bloom — peaks |

One hue, not a rainbow: the waterfall encodes a single ordered quantity
(power), so a hue change would imply a category boundary that does not exist.
The ramp is strictly monotonic in CIE L\* (0.8 → 97.1), which is the property
that makes "brighter" mean "more signal" everywhere on the scale; a
non-monotonic ramp reads as less signal wherever luminance dips. Implemented in
`src/components/Waterfall/colormap.ts` and enforced by its test.

Superseded the earlier `[dark blue → blue → cyan → green → yellow → red]`
recommendation, which was not monotonic in luminance (cyan is brighter than
green) and carried five hue changes.

**Frame rate**: target 25–30 fps. At fs=2.048 MHz and N=8192:
`T = 8192/2048000 ≈ 4ms per FFT`. Every frame between emits is FFT-processed and
accumulated into a **power average** (`10^(dB/10)` per bin, back to `10·log10(avg)`);
the averaged frame is emitted at ~25 fps. This provides ~5 dB SNR improvement over
single-frame snapshots (≈ sqrt(10) for 10 frames). The accumulator is flushed on
every retune so cross-frequency blending cannot occur. Do not skip frames — average them.

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

**De-emphasis filter (WBFM only)**: broadcast FM applies pre-emphasis before
transmission; demodulators must apply the inverse RC filter:
```
H(z) = (1 - e^(-1/τfs)) / (1 - e^(-1/τfs)·z^(-1))
```
Time constant τ by region: **75 µs (Americas — default)**, 50 µs (Europe / Japan).
Using the wrong τ causes audible tonal imbalance (~8 dB error at 10 kHz).
Change `DEEMPHASIS_TAU_S` in `src-tauri/src/dsp/demod/mod.rs` to match your region.

For all modes: single-stage decimation with a 65-tap windowed-sinc LPF.
The channel cutoff is set to half the user bandwidth, bounded by 90 % of
the baseband Nyquist. For NFM, AM and SSB the narrow channel bandwidth
leaves an ample guard band; for WBFM the 65-tap filter provides adequate
suppression for practical reception quality.

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

### DC-blocking IIR (SSB baseband)

A second-order Butterworth high-pass at 10 Hz is applied to complex baseband before
the Hilbert transform to eliminate I/Q DC bias introduced by the RTL-SDR's digitisation
path. RBJ cookbook coefficients (`Q = 1/√2`, `fs = 16 kHz`, `fc = 10 Hz`):

```
w0 = 2π · 10 / 16000
α  = sin(w0) / √2
b0 = b2 = (1 + cos w0) / (2(1 + α))
b1 = −(1 + cos w0) / (1 + α)
a1 = −2 cos w0 / (1 + α),   a2 = (1 − α) / (1 + α)
```

Applied independently to I and Q to preserve per-channel state.

### Group delay compensation (SSB)

The 65-tap audio LPF has group delay `(65 − 1) / 2 = 32` samples. Applying it to both
I and Q paths separately (before the Hilbert transform on Q) keeps both paths aligned:

```
Q path: audio_lpf (32 samples) + Hilbert FIR (64 samples) = 96 samples total
I path: audio_lpf (32 samples) + I delay buffer (64 samples) = 96 samples total
```

The `i_buf` length (`HILBERT_TAPS − 1) / 2 = 64`) is unchanged — it compensates only for
the Hilbert group delay, since the LPF delays cancel symmetrically between the paths.

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
| Rectangular | High (-13 dB) | Narrow | Never — severe leakage artifacts |
| Hann | Medium (-31 dB) | Medium | FIR filter design (sinc_lowpass_taps) |
| Blackman-Harris | Low (-92 dB) | Wide | **Default for waterfall/spectrum** |

**Default for display**: Blackman-Harris 4-coefficient (−92 dB sidelobes).
With Hann, a −20 dBFS carrier bleeds −51 dBFS sidelobe energy across the full
bandwidth, raising the apparent noise floor by 30 dB. Blackman-Harris reduces
that to −112 dBFS (invisible). Trade-off: main lobe is ~8 bins wide vs 4 for Hann.

Hann formula (used for FIR sinc kernel windowing only):
```
w[n] = 0.5·(1 - cos(2π·n / (N-1)))   for n = 0..N-1
```

Blackman-Harris formula (used for FFT display):
```
A = [0.35875, 0.48829, 0.14128, 0.01168]
w[n] = A[0] - A[1]·cos(2π·n/(N-1)) + A[2]·cos(4π·n/(N-1)) - A[3]·cos(6π·n/(N-1))
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
| SSB audio DAC overflow | Hilbert combine peaks at ±1.5 | tanh soft-clip before resampler |
| FFT size mismatch | N not matching buffer | Assert N == buffer size before FFT |
| Normalization drift | No reference level | Fix noise floor reference at startup |
| Ghost signal: a peak recedes as you tune toward it | Waterfall history keeps its painted columns while the axis is relabelled for the new centre; displaced by `new centre − old centre` | `retuneShiftPx` slides the history on every non-drag retune (§9.6). Drag already shifts it |
| Ghost signal, same symptom, history cleared | Not a fault in the chain: a synthetic carrier swept through live LO → mixer → FFT → crop lands within one bin at every centre (`ghost_sweep.rs`, `apparentFrequency.test.ts`). IQ-image mirror (slope −1) and fs/4 sign error (offset ≈ fs/2) are ruled out. Left to measure on hardware: a spur fixed relative to the LO, and queued old-centre IQ — at most `IQ_CHANNEL_CAPACITY` chunks + USB buffers, about 96 ms at 2.048 Msps | Compare with a second dongle or SDR#/GQRX at the same centres (issue #24) |

---

## 9. Display coordinate systems (bin ↔ Hz ↔ pixel)

Implemented in `src/viewport/spectrumViewport.ts`, `src/viewport/cellAxis.ts` and
`src/viewport/canvasSizing.ts`. Every overlay stacked on the spectrum reads
this section's transform from there rather than rebuilding it.

### 9.1 The three spaces

| Space | Unit | Owner |
|---|---|---|
| Bin | FFT bin index, 0 … N−1 after the shift of §2 step 6 | Rust (`N`), cropped for zoom in `Waterfall`'s `cropCenter` |
| Frequency | real Hz | the tuned centre + the visible span |
| Pixel | canvas x | one of three pixel spaces — see §9.4 |

### 9.2 Hz ↔ pixel

The visible span is the true one of §9.5, `span = fs · kept / N`. After the
fs/4 mixer of §1–3 the tuned signal sits at the centre of bin `N/2`; `minHz`
is the left edge of the first kept bin:

```
minHz = f + (start − N/2 − ½) · fs / N     (start, kept: §9.5)
maxHz = minHz + span
x     = (hz − minHz) / span · width
hz    = minHz + x / width · span
```

`x ↔ hz` is an exact round trip. Two relative forms drop the centre term:
`hzWidthToPx(w) = w/span · width` and `pxWidthToHz(p) = p/width · span`. Pan
gestures must use the relative form — an absolute `xToHz` inside a handler
that retunes on every move changes `minHz`, which changes the next move's
mapping, which is a runaway pan.

### 9.3 Bin ↔ pixel

Bin space never passes through Hz. The two directions are deliberately
asymmetric, because the two consumers need different things:

- `binLeftX(i)  = i / binCount · width` — a **point** map. The spectrum
  polyline needs one vertex per bin, placed at the bin's left edge.
- `xToBinIndex(x) = floor(x · binCount / width)` — a **cell** map. The
  waterfall row needs the bin *covering* a pixel column.

They compose to identity in the direction `xToBinIndex(binLeftX(i)) === i`,
and only that direction; the reverse lands within one bin width. `binCount`
is always an input, so `cropCenter` stays the sole owner of what is on screen.

Both directions come from one implementation, `src/viewport/cellAxis.ts`,
which knows only index / count / width. The scanner's band-activity strip
draws and hit-tests against the same map with its axis in scan-step index
over an arbitrary frequency list; it takes the clamped inverse, because its
x comes from a pointer and can land outside the strip.

### 9.4 The DPR rule

One rule: a canvas's backing store is its CSS size × an explicit
`pixelRatio`, rounded to whole device pixels, and its 2d context is
pre-transformed by that ratio so all drawing happens in CSS pixels.

The frequency axis, band guide and filter marker pass
`window.devicePixelRatio`. The two streaming canvases — the waterfall row and
the spectrum curve — pass **1**, the one documented exception: both the
per-pixel LUT loop and the `ImageData` row scale with backing-store width, and
`PERF.md §1` sets a ~1 ms/frame NO-GO threshold measured without DPR.

That exception is also why the waterfall's drag hit-test can treat
`canvas.width / rect.width` as 1. Three pixel spaces coexist and must not be
conflated: CSS pixels (overlays, under `setTransform(dpr, …)`), backing-store
pixels (the streaming pair), and `getBoundingClientRect().width` (hit-testing).

### 9.5 True span and the crop window

The overlays label the span the waterfall actually shows, not `fs / z`:

```
kept  = min(N, max(16, floor(N / z)))
span  = fs · kept / N
start = floor((N + 1 − kept) / 2)
minHz = f + (start − N/2 − ½) · fs / N
```

The tuned frequency is the *centre* of bin `N/2` after the FFT shift, so the
crop is placed around edge index `N/2 + ½`. `start` is the nearest whole bin;
when `N − kept` is even the residual is half a bin (125 Hz at N = 8192,
fs = 2.048 MHz), and `minHz` absorbs it so every overlay lands exactly on the
bins drawn. At z = 1 the span is `fs` and `minHz` sits half a bin below
`f − fs/2`. Nominal `fs / z` was off by up to ~0.5 % at z = 50 and put the crop
half a bin off centre (issue #17).

`cropWindow` in `src/viewport/spectrumViewport.ts` is the one owner of
`start`/`kept`; the waterfall crop and the viewport both call it.

### 9.6 Retune and the painted history

Waterfall rows are pixels: a row keeps the column it was painted at. When the
centre moves from `f₀` to `f₁` the history must slide by
`retuneShiftPx = round(hzWidthToPx(f₀ − f₁))` or every old streak is
relabelled `f₁ − f₀` away from its true frequency. A drag shifts the canvas
itself as it goes; every other retune (click, keyboard, typed, scanner) goes
through the `frequencyHz` effect in `Waterfall`. Content shifted past an edge
is lost; the exposed strip is background.
