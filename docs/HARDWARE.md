# HARDWARE.md — RTL-SDR Hardware Reference

## Table of contents
1. [RTL-SDR fundamentals](#1-rtl-sdr-fundamentals)
2. [librtlsdr binding strategy](#2-librtlsdr-binding-strategy)
3. [Device configuration parameters](#3-device-configuration-parameters)
4. [Sampling and tuning constraints](#4-sampling-and-tuning-constraints)
5. [Gain control](#5-gain-control)
6. [Known hardware issues](#6-known-hardware-issues)

---

## 1. RTL-SDR fundamentals

The RTL-SDR is based on the **Realtek RTL2832U** chip paired with a tuner
(most commonly R820T/R820T2). It functions as a direct-sampling or
quadrature-sampling USB SDR receiver.

**Supported frequency range**: 500 kHz – 1.75 GHz (with R820T2 tuner).
Below 500 kHz requires direct-sampling mode (hardware mod or specific dongles).

**ADC resolution**: 8-bit (256 levels per I and Q channel).
This limits dynamic range to approximately 48 dB theoretically.
Practical dynamic range: ~50–60 dB depending on gain settings.

**Output format**: interleaved 8-bit unsigned integers.
Conversion to complex float: `I = (raw_I / 127.5) - 1.0`, same for Q.
Range after conversion: `[-1.0, 1.0]`.

---

## 2. librtlsdr binding strategy

**Approach**: direct FFI binding via `rtlsdr-rs` crate or raw `unsafe` FFI.
Do NOT use `rtl_tcp` daemon — see `CLAUDE.md` for rationale.

**Crate**: use `rtlsdr-rs` as primary. If it lacks needed features,
fall back to raw `librtlsdr` FFI using the C header directly.

**Initialization sequence**:
```
1. rtlsdr_get_device_count() → verify at least 1 device
2. rtlsdr_open(&dev, device_index) → open handle
3. rtlsdr_set_sample_rate(dev, sample_rate)
4. rtlsdr_set_center_freq(dev, center_freq_hz)
5. rtlsdr_set_tuner_gain_mode(dev, 0) → 0=auto, 1=manual
6. rtlsdr_reset_buffer(dev) → mandatory before streaming
7. rtlsdr_read_async(dev, callback, ctx, 0, buffer_size) → start stream
```

**Shutdown sequence**:
```
1. rtlsdr_cancel_async(dev) → signal callback to stop
2. rtlsdr_close(dev) → release handle
```

---

## 3. Device configuration parameters

| Parameter | Recommended default | Notes |
|---|---|---|
| Sample rate | 2.048 MHz | Stable, good bandwidth |
| Center frequency | User-set | Offset by fs/4 from signal — see DSP.md §1 |
| Buffer size | 16384 bytes (16 KB) | ~8ms at 2.048 MHz |
| Tuner gain mode | Auto (0) | Expose manual override in UI |
| PPM correction | 0 | Expose as user setting for calibration |
| Direct sampling | Off | Only needed for HF below 500 kHz |

**Buffer size formula**: `buf_size = sample_rate × bytes_per_sample × duration_s`
At 2.048 MHz, 2 bytes/sample (I+Q), 8ms: `2048000 × 2 × 0.008 = 32768 bytes`.

---

## 4. Sampling and tuning constraints

**Stable sample rates** (avoid rates that cause dropped samples):
- 225 kHz, 900 kHz, 1.024 MHz, 1.4 MHz, 1.8 MHz
- **2.048 MHz** ← recommended default
- 2.4 MHz ← maximum stable on most hardware
- Above 3.2 MHz: expect dropped samples

**Tuning resolution**: RTL-SDR tunes in steps. Actual center frequency
may differ slightly from requested. Always read back:
`actual_freq = rtlsdr_get_center_freq(dev)`

**Minimum tunable frequency**: ~500 kHz with standard R820T2.
Below this requires direct-sampling mode (not in V1 scope).

**Maximum stable frequency**: ~1.75 GHz with R820T2.
Above 1.1 GHz: expect increased phase noise.

---

## 5. Gain control

RTL-SDR exposes two gain stages:
1. **Tuner gain** (RF amplifier, R820T2) — main control, in tenths of dB
2. **IF gain** (RTL2832U digital gain) — secondary, rarely needed

**Auto gain**: hardware AGC. Good for general use, may compress strong signals.

**Manual gain**: `rtlsdr_set_tuner_gain(dev, gain_tenths_db)`
Available gain steps are hardware-specific. Query with:
`rtlsdr_get_tuner_gains(dev, gains_array)`

**Typical gain range**: 0 to ~50 dB in discrete steps (~1–2 dB per step).

**Gain strategy for UI**:
- Default: auto
- Manual: expose dB slider using queried gain steps
- Warn user if signal appears clipped (ADC saturation)

---

## 6. Known hardware issues

| Issue | Cause | Mitigation |
|---|---|---|
| Device not found | Driver conflict (Windows) | Require Zadig WinUSB driver install |
| USB drops at high sample rate | USB 2.0 bandwidth limit | Cap at 2.4 MHz |
| Frequency drift | Cheap oscillator, temperature | Expose PPM correction setting |
| DC spike at center | LO leakage (RTL2832U) | Offset center freq — see DSP.md §1 |
| Gain steps not found | Device index wrong | Always query `get_device_count` first |
| Callback not called | `reset_buffer` skipped | Always call before `read_async` |
| Click/pop on start | Buffer not primed | Discard first 2–3 buffers |

**Platform notes**:
- Linux: requires `rtlsdr` udev rules or running as root
- macOS: librtlsdr installable via Homebrew
- Windows: requires Zadig to replace default driver with WinUSB
