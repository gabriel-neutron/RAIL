# SIGNALS.md — Signal Types and Capture Format Reference

## Table of contents
1. [SigMF format specification](#1-sigmf-format-specification)
2. [Capture session schema](#2-capture-session-schema)
3. [Signal type taxonomy](#3-signal-type-taxonomy)
4. [Frequency domains in RAIL's antenna range](#4-frequency-domains-in-rails-antenna-range)

---

## 1. SigMF format specification

RAIL uses **SigMF (Signal Metadata Format)** as its capture standard.
Spec: https://github.com/sigmf/SigMF

A SigMF capture consists of two files:
- `<name>.sigmf-data` — raw IQ samples (binary)
- `<name>.sigmf-meta` — JSON metadata file

### sigmf-meta structure

```json
{
  "global": {
    "core:datatype": "cf32_le",
    "core:sample_rate": 2048000,
    "core:version": "1.0.0",
    "core:description": "User-provided description",
    "core:author": "RAIL",
    "rail:center_frequency_hz": 100000000,
    "rail:tuner_gain_db": 30,
    "rail:demod_mode": "FM",
    "rail:filter_bandwidth_hz": 200000
  },
  "captures": [
    {
      "core:sample_start": 0,
      "core:datetime": "2024-01-01T12:00:00Z",
      "core:frequency": 100000000
    }
  ],
  "annotations": []
}
```

**Datatype `cf32_le`**: complex float32, little-endian.
Each sample = 8 bytes (4 bytes I + 4 bytes Q).

**Custom namespace `rail:`**: use for RAIL-specific fields not in core SigMF spec.

### sigmf-data format

Raw binary: interleaved float32 complex samples.
`[I0, Q0, I1, Q1, ..., In, Qn]`

Sample values are normalized to `[-1.0, 1.0]` (converted from raw RTL-SDR u8).

---

## 2. Capture session schema

A session groups multiple captures with shared metadata.
Stored as `<session_name>.rail-session.json` alongside SigMF files.

```json
{
  "session_id": "uuid-v4",
  "created_at": "ISO-8601 datetime",
  "label": "User-defined label",
  "notes": "Free text annotation",
  "captures": [
    {
      "capture_id": "uuid-v4",
      "sigmf_meta_path": "relative/path/to/file.sigmf-meta",
      "sigmf_data_path": "relative/path/to/file.sigmf-data",
      "type": "iq_clip | audio_recording | waterfall_screenshot",
      "duration_ms": 5000,
      "frequency_hz": 100000000,
      "mode": "FM",
      "signal_type_guess": "WBFM | NBFM | AM | unknown",
      "tags": ["broadcast", "stereo"]
    }
  ]
}
```

**Audio recordings**: saved as WAV (PCM float32, 44100 Hz, mono).
Not SigMF — SigMF is for raw IQ only.

**Waterfall screenshots**: PNG, timestamped filename.

---

## 3. Signal type taxonomy

For V1, RAIL uses a simple signal classification vocabulary.
Used in session annotations and (later) automated detection.

| Label | Description | Typical bandwidth |
|---|---|---|
| `WBFM` | Wideband FM broadcast | 150–200 kHz |
| `NBFM` | Narrowband FM (voice, PMR) | 5–25 kHz |
| `AM` | Amplitude modulation | 3–30 kHz |
| `USB` | Upper sideband SSB | 3 kHz |
| `LSB` | Lower sideband SSB | 3 kHz |
| `CW` | Morse code (carrier wave) | <1 kHz |
| `digital_narrowband` | Unknown digital, narrow | <25 kHz |
| `digital_wideband` | Unknown digital, wide | >25 kHz |
| `burst` | Short-duration unknown | varies |
| `carrier` | Unmodulated carrier | near 0 |
| `unknown` | No classification | — |

---

## 4. Frequency domains in RAIL's antenna range

RAIL's supported antenna range: ~100 kHz – 1.75 GHz.

| Band | Range | Common signals |
|---|---|---|
| MF | 300 kHz – 3 MHz | AM broadcast, maritime |
| HF | 3 – 30 MHz | Shortwave, amateur, VOLMET |
| VHF low | 30 – 88 MHz | Paging, military |
| FM broadcast | 87.5 – 108 MHz | WBFM stereo radio |
| VHF high | 108 – 174 MHz | Aviation (VOR/ILS), amateur |
| UHF | 300 – 900 MHz | PMR446, trunked radio, ADS-B (1090 MHz*) |
| L-band | 1 – 1.75 GHz | GPS (1575 MHz*), Inmarsat |

*Note: 1090 MHz (ADS-B) and 1575 MHz (GPS L1) are near or above the reliable
range of most RTL-SDR + R820T2 setups. Reception is possible but not guaranteed.
Do not design features that depend on these without hardware verification.

**Priority for V1 demos**: FM broadcast band (87.5–108 MHz).
Signals are strong, demodulation is well-understood, and results are immediately
audible — best for showcasing the tool without specialized knowledge.
