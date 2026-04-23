# RTL-SDR Hardware Reference

## Table of contents
1. RTL-SDR fundamentals
2. librtlsdr binding strategy
3. Device configuration parameters
4. Sampling and tuning constraints
5. Gain control
6. Known hardware issues

---

## 1. RTL-SDR fundamentals

The NESDR SMArt v5 is based on the Realtek RTL2832U demodulator/USB interface IC paired with the R820T2/R860 tuner IC. It operates in two modes depending on the target frequency:

- **Quadrature (IQ) sampling mode**: 25 MHz – 1.75 GHz (standard operation)
- **Direct sampling mode**: 100 kHz – 25 MHz (HF reception, built-in, no hardware mod required)

**Key hardware characteristics:**

| Parameter | Value |
|---|---|
| Frequency range | 100 kHz – 1.75 GHz |
| Demodulator IC | RTL2832U |
| Tuner IC | R820T2 / R860 |
| ADC resolution | 7-bit (128 levels per I and Q channel) |
| TCXO stability | 0.5 PPM (ultra-low phase noise) |
| Antenna input | Female SMA, 50 Ω |
| USB interface | USB Type-A |
| Enclosure | Black brushed aluminum with integrated custom heatsink |
| Certifications | FCC, CE, IC |

**ADC and dynamic range:**

The RTL2832U uses a 7-bit ADC (128 levels per I and Q channel). This limits theoretical dynamic range to approximately 42 dB. Practical dynamic range is 50–60 dB depending on gain settings and environmental conditions.

Output format: interleaved 8-bit unsigned integers (zero-padded from the 7-bit ADC).
Conversion to complex float: `I = (raw_I / 127.5) - 1.0`, same for Q. Range after conversion: `[-1.0, 1.0]`.

**Improvements over RTL-SDR v3:**
- HF SNR improved by up to 15 dB
- VHF & UHF SNR improved by up to 6 dB
- Tuning accuracy improved by an average of 4×
- Frequency range extended down to 100 kHz via native direct sampling

---

## 2. librtlsdr binding strategy

Approach: direct FFI binding via `rtlsdr-rs` crate or raw unsafe FFI. Do NOT use rtltcp daemon (see `CLAUDE.md` for rationale).

Crate: use `rtlsdr-rs` as primary. If it lacks needed features, fall back to raw `librtlsdr` FFI using the C header directly.

**Initialization sequence:**
1. `rtlsdr_get_device_count()` — verify at least 1 device
2. `rtlsdr_open(&dev, device_index)` — open handle
3. `rtlsdr_set_sample_rate(dev, sample_rate)` — set sample rate
4. `rtlsdr_set_center_freq(dev, center_freq_hz)` — set center frequency
5. `rtlsdr_set_tuner_gain_mode(dev, 0)` — 0 = auto, 1 = manual
6. `rtlsdr_reset_buffer(dev)` — **mandatory** before streaming
7. `rtlsdr_read_async(dev, callback, ctx, 0, buffer_size)` — start stream

**For HF (below 25 MHz) — direct sampling mode:**
- Call `rtlsdr_set_direct_sampling(dev, 2)` after open, before streaming
  - Value `1` = I-branch, `2` = Q-branch (Q-branch recommended for NESDR SMArt v5)
- Disable tuner gain: `rtlsdr_set_tuner_gain_mode(dev, 0)`
- Note: tuner gain has no effect in direct sampling mode

**Shutdown sequence:**
1. `rtlsdr_cancel_async(dev)` — signal callback to stop
2. `rtlsdr_close(dev)` — release handle

---

## 3. Device configuration parameters

| Parameter | Recommended default | Notes |
|---|---|---|
| Sample rate | 2.048 MHz | Stable, good bandwidth |
| Center frequency | User-set | Offset by fs/4 from signal (see `DSP.md`) |
| Buffer size | 16384 bytes (16 KB) | 8ms at 2.048 MHz |
| Tuner gain mode | Auto (0) | Expose manual override in UI |
| PPM correction | 0 | 0.5 PPM TCXO — very stable; expose as user setting for fine calibration |
| Direct sampling | Off (quadrature) | Set to Q-branch (2) for HF below 25 MHz |

**Buffer size formula:**

```
buf_size = sample_rate × bytes_per_sample × duration_s
```

At 2.048 MHz, 2 bytes/sample (IQ), 8 ms:
`2,048,000 × 2 × 0.008 = 32,768 bytes`

---

## 4. Sampling and tuning constraints

**Stable sample rates** (avoid rates that cause dropped samples):
- 225 kHz, 900 kHz, 1.024 MHz, 1.4 MHz, 1.8 MHz
- **2.048 MHz** — recommended default
- 2.4 MHz — maximum stable on most hardware
- Above 3.2 MHz — expect dropped samples (3.2 MSPS is the hardware ceiling)

**Tuning resolution:**
The RTL-SDR tunes in discrete steps. The actual center frequency may differ slightly from the requested value. Always read back: `actual_freq = rtlsdr_get_center_freq(dev)`.

**Frequency range by mode:**

| Mode | Range | Notes |
|---|---|---|
| Quadrature (IQ) | 25 MHz – 1.75 GHz | Standard mode |
| Direct sampling (Q-branch) | 100 kHz – 25 MHz | Native on v5, no hardware mod required |

Above 1.1 GHz expect increased phase noise.

**HF note:** Although direct sampling on the NESDR SMArt v5 is significantly better than other RTL-SDRs (up to +15 dB HF SNR), using an upconverter is still recommended for demanding HF work to avoid the DC spike and alias artifacts inherent to direct sampling.

---

## 5. Gain control

The NESDR SMArt v5 exposes two gain stages:

1. **Tuner gain** — R820T2/R860 RF amplifier. Main control, expressed in tenths of dB.
2. **IF gain** — RTL2832U digital gain. Secondary stage, rarely needed.

**Gain range:** 0 to 49.6 dB (in discrete steps, hardware-specific).

**Gain strategy for UI:**
- Default: auto (AGC)
- Manual: expose dB slider using queried gain steps
- Warn user if signal appears clipped (ADC saturation)
- Note: in direct sampling mode (HF), tuner gain has no effect — IF gain or AGC applies

**Querying available gain steps:**
```rust
rtlsdr_get_tuner_gains(dev, &mut gains_array)
```
Typical step resolution: ~1–2 dB increments across the 0–49.6 dB range.

**Auto gain:** suitable for general use; may compress strong signals in dense RF environments. Prefer manual gain when doing signal level measurements or decoding weak signals.

---

## 6. Known hardware issues

| Issue | Cause | Mitigation |
|---|---|---|
| Device not found | Driver conflict (Windows) | Require Zadig WinUSB driver install |
| USB drops at high sample rate | USB 2.0 bandwidth limit | Cap at 2.4 MHz |
| Frequency drift | Minimal — 0.5 PPM TCXO | Expose PPM correction setting for fine-tuning |
| DC spike at center | LO leakage, RTL2832U | Offset center freq (see `DSP.md`) |
| Gain steps not found | Device index wrong | Always query `get_device_count()` first |
| Callback not called | `reset_buffer` skipped | Always call before `read_async` |
| Click/pop on start | Buffer not primed | Discard first 2–3 buffers |
| HF alias artifacts | Direct sampling architecture | Use upconverter for critical HF work |
| Weak HF signal | Direct sampling requires matched antenna | Use a long-wire antenna or DIY dipole for HF; a 1:9 balun (e.g. Balun One Nine) is strongly recommended |

**Platform notes:**

| Platform | Requirement |
|---|---|
| Linux | Requires `rtlsdr` udev rules or running as root |
| macOS | `librtlsdr` installable via Homebrew |
| Windows | Requires Zadig to replace default driver with WinUSB |
