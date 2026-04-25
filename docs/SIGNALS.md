# SIGNALS.md — Signal Types and Capture Format Reference

## Table of contents
1. [SigMF format specification](#1-sigmf-format-specification)
2. [Capture session schema](#2-capture-session-schema)
3. [Signal type taxonomy](#3-signal-type-taxonomy)
4. [Receivable signals by frequency band](#4-receivable-signals-by-frequency-band)
5. [Classification heuristics reference](#5-classification-heuristics-reference)
6. [Classifier design note](#6-classifier-design-note)

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

RAIL uses a fixed vocabulary for signal classification.
Used in session annotations and automated detection (Phase 10+).

| Label | Description | Typical BW | Modulation family |
|---|---|---|---|
| `WBFM` | Wideband FM broadcast | 150–200 kHz | Analog FM |
| `NBFM` | Narrowband FM (voice, PMR, amateur) | 5–25 kHz | Analog FM |
| `AM` | Amplitude modulation (aviation, broadcast) | 3–30 kHz | Analog AM |
| `USB` | Upper sideband SSB | ~3 kHz | Analog SSB |
| `LSB` | Lower sideband SSB | ~3 kHz | Analog SSB |
| `CW` | Morse code | < 1 kHz | On/off keying |
| `ADS-B` | Aircraft transponder (Mode S) | ~1 MHz burst | Digital OOK-PPM |
| `AIS` | Maritime ship transponder | ~25 kHz | Digital GMSK |
| `APRS` | Amateur packet reporting (AX.25) | ~16 kHz | Digital AFSK 1200 |
| `NOAA-APT` | Weather satellite image | ~40 kHz | Analog FM + 2400 Hz sub |
| `OOK` | On/off keying (ISM remotes, sensors) | 1–100 kHz | Digital OOK/ASK |
| `POCSAG` | Paging protocol | ~12.5 kHz | Digital FSK |
| `digital_narrowband` | Unknown digital, narrow | < 25 kHz | Unknown digital |
| `digital_wideband` | Unknown digital, wide | > 25 kHz | Unknown digital |
| `burst` | Short-duration unknown | varies | Unknown |
| `carrier` | Unmodulated carrier | near 0 | None |
| `unknown` | No classification | — | — |

---

## 4. Receivable signals by frequency band

**Hardware reference**: RTL-SDR with R820T2 tuner.
Reliable coverage: **~24 MHz – 1766 MHz** (R820T2).
Practical sweet spot: **50 MHz – 1.5 GHz** (best sensitivity).
Below 24 MHz requires a direct-sampling hardware modification — not supported by RAIL.

Signal entries marked **[RAIL: done]** are already implemented.
Entries marked **[RAIL: Phase N]** indicate the planned implementation phase.

---

### 4.1 FM Broadcast — 87.5–108 MHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| WBFM stereo | 87.5–108 MHz (200 kHz steps) | ~200 kHz | FM (±75 kHz dev) | Excellent | done |
| RDS/RBDS | 57 kHz subcarrier on FM stations | <5 kHz | BPSK subcarrier | Excellent | not planned |

**Notes**: Strongest signals in the entire RTL-SDR range. Primary demo band.
RDS carries station name, song info, and traffic data — decoding it requires a
57 kHz subcarrier demodulator + BPSK decoder, not planned for V2.

---

### 4.2 Aeronautical Navigation — 108–118 MHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| VOR beacon | 108–118 MHz (50 kHz steps) | ~50 kHz | AM + 30 Hz + 9960 Hz sub | Good | not planned |
| ILS Localizer | 108.1–111.95 MHz | ~50 kHz | AM with 90/150 Hz tones | Good | not planned |

**Notes**: AM modulation family; decodable with AM demod but meaningful decoding
requires tone analysis (30 Hz, 9960 Hz, 90/150 Hz). Classification hint: AM signal
in 108–118 MHz range → likely VOR or ILS.

---

### 4.3 Aviation Voice (ATC) — 118–137 MHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| ATC voice | 118–136.975 MHz (25 kHz steps) | ~8–12 kHz | AM (DSB-LC) | Excellent | done |
| ATIS | Varies per airport | ~8 kHz | AM | Excellent | done |
| VOLMET | 127.0, 128.6 MHz (varies by region) | ~8 kHz | AM | Good | done |

**Notes**: Standard AM demodulator works directly. Very rewarding band —
aircraft and ground communications are clearly audible near any airport.
Classification hint: AM signal in 118–137 MHz → very likely aviation voice.

---

### 4.4 Weather Satellites — 137–138 MHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| NOAA 15 APT | 137.620 MHz | ~40 kHz | FM + 2400 Hz sub | Good (overhead pass) | not planned |
| NOAA 18 APT | 137.912 MHz | ~40 kHz | FM + 2400 Hz sub | Good (overhead pass) | not planned |
| NOAA 19 APT | 137.100 MHz | ~40 kHz | FM + 2400 Hz sub | Good (overhead pass) | not planned |

**Notes**: Requires a directional (V-dipole) antenna and passes only last ~10 min.
APT decoding (image reconstruction from audio tones) is complex and not in scope.
Classification hint: FM signal at exactly 137.100/137.620/137.912 MHz → NOAA-APT.

---

### 4.5 VHF Amateur Radio (2m) — 144–146 MHz (EU) / 144–148 MHz (US)

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| FM voice (repeaters) | 144.300–146.000 MHz (EU) | 12.5–25 kHz | NBFM | Excellent | Phase 8 |
| SSB voice (DX) | 144.100–144.400 MHz | ~3 kHz | USB | Good | Phase 8 |
| APRS | 144.800 MHz (EU) / 144.390 MHz (US) | ~16 kHz | AFSK 1200 baud | Excellent | not planned |
| CW | 144.000–144.150 MHz | < 1 kHz | CW | Good | Phase 8 |

**Notes**: APRS (Automatic Packet Reporting System) broadcasts GPS positions,
weather, and text messages over AX.25 packet radio. The audio sounds like
a modem at 1200 baud. Decoding requires an AX.25 demodulator — not in V2 scope,
but detection (identifying the characteristic AFSK tones) is feasible in Phase 10.
Classification hint: NBFM signal at 144.800 MHz → very likely APRS.

---

### 4.6 Maritime VHF — 156–174 MHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| VHF voice | All channels, ch16 = 156.800 MHz | 12.5–25 kHz | NBFM | Excellent | Phase 8 |
| AIS channel 1 | 161.975 MHz | ~25 kHz | GMSK 9600 baud | Excellent | not planned |
| AIS channel 2 | 162.025 MHz | ~25 kHz | GMSK 9600 baud | Excellent | not planned |

**Notes**: AIS (Automatic Identification System) continuously broadcasts ship
identity (MMSI), position, speed, and course. Ships within ~40 km are typically
audible. AIS decoding requires GMSK demodulation + NMEA sentence parsing —
the decoded data (ship names, positions) is highly demo-worthy but not in V2 scope.
Classification hint: GMSK signal at 161.975 or 162.025 MHz → very likely AIS.

---

### 4.7 NOAA Weather Radio — 162.400–162.550 MHz (US)

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| NWR broadcast | 162.400/162.425/162.450/162.475/162.500/162.525/162.550 MHz | ~25 kHz | NBFM | Excellent (US only) | Phase 8 |

---

### 4.8 Paging — 138–174 MHz (varies by region)

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| POCSAG | 153.050 MHz (FR), 148–174 MHz (varies) | ~12.5 kHz | FSK 512/1200/2400 baud | Good | not planned |
| FLEX | Similar range | ~12.5 kHz | 4-FSK | Good | not planned |

**Notes**: Pager traffic (including hospital, emergency services) is FSK-modulated.
Detection is straightforward (regular burst pattern, FSK). Decoding POCSAG requires
an FSK demodulator + POCSAG framing parser — not in V2 scope.

---

### 4.9 DAB+ Digital Radio — 174–240 MHz (Europe)

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| DAB+ ensemble | 174–240 MHz (1.536 MHz blocks) | ~1.5 MHz | OFDM | Good (EU) | not planned |

**Notes**: OFDM wideband signal. Looks like a flat-topped noise block ~1.5 MHz wide
on the waterfall. Decoding is complex (OFDM demodulation + AAC-LC audio). Not in scope.
Classification hint: flat-spectrum block ~1.5 MHz wide in 174–240 MHz → likely DAB+.

---

### 4.10 ISM 433 MHz — 433.050–434.790 MHz (EU)

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| OOK remotes | 433.920 MHz (most common) | 1–100 kHz | OOK/ASK | Excellent | Phase 10 |
| FSK sensors | 433.920 MHz ± spread | 5–50 kHz | FSK | Excellent | Phase 10 |
| LoRa IoT | 433.175/433.375/433.575 MHz | ~125/250 kHz | CSS (LoRa) | Good | not planned |

**Notes**: The 433 MHz ISM band is extremely active. Every car remote, wireless weather
station, doorbell, tire pressure sensor, and smart plug operates here. Signals are
short bursts (OOK) or continuous FSK. Highly visible on the waterfall.
No decoding required for detection — the bursts are unmistakable visually.
Classification hint: short OOK burst at 433.920 MHz → very likely ISM remote/sensor.

---

### 4.11 PMR446 (EU) — 446.000–446.200 MHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| PMR446 voice | 446.00625–446.19375 MHz (8 ch, 25 kHz steps) | 12.5 kHz | NBFM | Excellent | Phase 8 |

**Notes**: License-free walkie-talkies. Very common in warehouses, events, hiking.
Equivalent to FRS (462–467 MHz) in the US.

---

### 4.12 UHF Amateur Radio (70cm) — 430–440 MHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| FM voice (repeaters) | 430–440 MHz | 12.5–25 kHz | NBFM | Excellent | Phase 8 |
| Digital (DMR, Fusion) | Various | 12.5 kHz | Digital NBFM-like | Good | not planned |

---

### 4.13 ISM 868 MHz (EU) / 915 MHz (US)

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| LoRa IoT | 863–870 MHz (EU) | 125/250/500 kHz | CSS (LoRa) | Good | not planned |
| Sigfox | 868.1 MHz | ~100 Hz (very narrow) | DBPSK | Good | not planned |
| Smart meters (M-Bus) | 868.3/869.525 MHz | ~100 kHz | FSK/GFSK | Good | not planned |

**Notes**: LoRa signals look like a chirp sweep on the waterfall — characteristic
rising or falling tone sweeping the full bandwidth. Unmistakable once seen.
Not planned for decoding in V2 but detectable (wideband digital).
Classification hint: chirp pattern in 863–870 MHz → likely LoRa.

---

### 4.14 ADS-B — 1090 MHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| Mode S / ADS-B | 1090 MHz (fixed) | ~1 MHz burst | OOK pulse-position (PPM) | Good–Excellent | not planned |

**Notes**: **ADS-B is the most demo-worthy signal in the entire RTL-SDR range.**
Every commercial aircraft broadcasts its ICAO address, GPS position, altitude,
speed, and callsign in the open (Mode S squitter). The signal is at exactly
1090 MHz, uses OOK-PPM at 1 Mbps, and is receivable with a simple whip antenna
over a ~200 km radius. Decoding requires bit-level OOK demodulation + Mode S
frame parsing — well-documented (reference: dump1090) but not in V2 scope.
Classification hint: OOK burst at 1090 MHz → unambiguous ADS-B.
The R820T2 tuner performs adequately at 1090 MHz but sensitivity is reduced
compared to 300–900 MHz; a filtered 1090 MHz antenna improves results significantly.

---

### 4.15 L-band — 1.2–1.75 GHz

| Signal | Center freq | BW | Modulation | Reception | RAIL |
|---|---|---|---|---|---|
| GPS L1 (C/A code) | 1575.420 MHz | ~2 MHz | BPSK spread spectrum | Very weak | not planned |
| GPS L2 | 1227.600 MHz | ~20 MHz | BPSK | Very weak | not planned |
| Inmarsat voice | 1525–1559 MHz | varies | Various | Weak | not planned |

**Notes**: GPS signals are below the noise floor without a dedicated LNA + patch
antenna. The R820T2 is at the edge of its range here. Not actionable for RAIL.

---

## 5. Classification heuristics reference

> All classifier logic in Rust must cite this section, not re-explain the rules.
> See also DSP.md §2 (FFT/magnitude) and DSP.md §4 (demodulation) for DSP primitives.

The classifier uses a **three-path dispatch** based on how many frequency-prior
candidates are available for the tuned frequency. The frequency prior is the
primary source of truth; spectral analysis only runs when the prior is ambiguous
or absent. This design avoids false cycling on single-band frequencies (FM broadcast,
aviation, maritime).

---

### 5.1 Bandwidth measurement

Occupied bandwidth is measured by walking outward from the peak bin until power
drops below `noise_floor + 6 dB` (not −3 dB from the peak, which is falsely
narrow for WBFM where the carrier is much stronger than its sidebands).
Noise floor is estimated as the **median** of all non-DC bins; this is a robust
estimator that tracks the true noise level regardless of spectral occupancy.

| Occupied BW | `BwFamily` |
|---|---|
| > 150 kHz | `Wideband` |
| 25–150 kHz | `Narrowband` |
| 3–25 kHz | `Voice` |
| < 3 kHz | `Narrow` |

Minimum SNR thresholds:
- `MIN_PEAK_SNR_DB = 10 dB`: minimum to detect a signal at all.
- `MIN_CONFIRM_SNR_DB = 20 dB`: minimum to populate `confirmed`. Below this,
  candidates are still returned but `confirmed` is `null`.

---

### 5.2 Modulation discrimination

Only computed for the multi-candidate path (§5.3) — not on every emission.

**AM vs FM (envelope variance)**
- FM signals: envelope nearly constant → low variance.
- AM signals: envelope tracks modulation → high variance.
- Threshold: `envelope_variance > 0.15` (normalized) → AM family.

**Sideband asymmetry (USB / LSB)**
- SSB: one sideband present, other absent. Power ratio upper vs lower half.
- Threshold raised to **15 dB** (vs a naïve 10 dB) to account for measurement
  noise in short 4 ms IQ windows. RTL-SDR cannot reliably distinguish SSB from
  NFM below this threshold.

---

### 5.3 Frequency prior and three-path dispatch

The frequency prior provides mode wire-name candidates for known bands.
These are always returned in `candidates` regardless of signal strength.

| Frequency | Candidates |
|---|---|
| 87.5–108 MHz | `FM` |
| 108–137 MHz | `AM` (VOR/ILS + aviation voice) |
| 137.100 / 137.620 MHz ± 5 kHz | `NFM` (NOAA-APT) |
| 144.800 MHz ± 10 kHz | `NFM` (APRS) |
| 144–146 MHz | `NFM`, `USB`, `CW` (2m amateur) |
| 156–174 MHz | `NFM` (maritime VHF) |
| 161.975 / 162.025 MHz ± 5 kHz | `NFM` (AIS) |
| 430–440 MHz | `NFM`, `USB` (70cm amateur) |
| 433.920 MHz ± 200 kHz | `AM`, `NFM` (ISM 433) |
| 446 MHz ± 100 kHz | `NFM` (PMR446) |
| 1090 MHz ± 500 kHz | _(none — ADS-B, no audio mode)_ |

**Three-path dispatch** (applied when SNR ≥ `MIN_CONFIRM_SNR_DB`):

| Candidates | Path | `confirmed` source |
|---|---|---|
| 0 (unknown band) | `broad_classify()` | `BwFamily` + `is_am_family`; never emits SSB/CW without prior |
| 1 (e.g. FM broadcast, aviation, maritime) | Trust prior directly | `candidates[0]`; no spectral analysis at all |
| ≥ 2 (e.g. 2m amateur) | `pick_from_candidates()` | First-match: FM→AM→USB→LSB→CW→NFM within candidate set |

The single-prior path eliminates false cycling on bands with a single
definitive mode — no amount of IQ content will change the result.

---

### 5.4 Classifier output contract

The Rust DSP task emits a `signal-classification` JSON Tauri event at ~2 Hz.
IPC payload:

```json
{
  "confirmed": "FM",
  "candidates": ["FM"],
  "reason": "BW=196kHz, var=0.012, asym=0.2dB, SNR=42.1dB @ 98.000MHz"
}
```

- `confirmed`: wire-name of the spectrally confirmed mode, or `null` when SNR
  is too low or the signal type has no selectable mode.
  Maps to a **green** ModeSelector button.
- `candidates`: wire-names from the frequency prior; always populated for known
  bands regardless of signal strength. Map to **yellow** buttons.
- `reason`: human-readable diagnostic string; shown as a tooltip.

**Rule**: when `confirmed == null`, do not apply any auto-mode suggestion.
The green button must not light up.

> **TODO**: The classifier heuristics need real-world validation across a wider
> range of bands, hardware dongles, and propagation conditions. The asymmetry
> threshold (15 dB) and envelope variance threshold (0.15) were set analytically
> from synthetic IQ; they may require tuning based on field measurements.
> See `src-tauri/src/dsp/classifier.rs` for the implementation.

---

### 5.5 Signals deferred beyond V2

The following signals are detectable with RTL-SDR but require protocol-specific
decoders beyond RAIL's V2 scope. Do not implement decoders for these in Phases 7–11.
They are listed here so that Phase 10 classification can correctly label them
without attempting to decode them.

| Signal | Why deferred |
|---|---|
| ADS-B (full decode) | Requires Mode S bit parser, ICAO database |
| AIS (full decode) | Requires GMSK demod + NMEA parser |
| APRS (full decode) | Requires AX.25 demod + APRS parser |
| DAB+ | Requires OFDM + AAC-LC decoder |
| LoRa | Requires CSS demodulation, proprietary framing |
| POCSAG | Requires FSK demod + POCSAG frame parser |
| RDS | Requires 57 kHz subcarrier demod + BPSK |
| NOAA APT (image) | Requires 2400 Hz tone sync + image reconstruction |
| GPS | Below noise floor, requires LNA + patch antenna |

---

## 6. Classifier design note

### Why the frequency prior runs first

Most SDR classifiers run spectral analysis on every frame and use frequency as a
tiebreaker. RAIL inverts this: the frequency prior runs first, and spectral analysis
only executes when the prior returns more than one candidate.

The reason is false-positive suppression on single-band frequencies. FM broadcast
(87.5–108 MHz) has exactly one valid mode: WBFM. Running envelope variance and
sideband asymmetry tests on every emission from that band produces cycling — the
carrier momentarily looks like AM during a fade, or the stereo pilot makes the
spectrum look asymmetric. The single-prior path eliminates this entirely: one
candidate means trust the prior directly, no analysis runs.

Spectral analysis is reserved for the multi-candidate case, where it actually adds
information. The 2m amateur band (144–146 MHz) carries FM voice, SSB DX, APRS,
and CW simultaneously; a frequency lookup returns all four candidates. Only then
do envelope variance and sideband asymmetry tests determine which one is present.

### Why the asymmetry threshold is 15 dB

The SSB asymmetry threshold is set at 15 dB, roughly 50% higher than a naïve
analytical derivation would suggest. The reason is measurement noise: 4 ms IQ
windows over an RTL-SDR dongle carry enough phase jitter and I/Q imbalance to
produce 5–10 dB of apparent sideband asymmetry on what is actually a symmetric
NFM signal. A 10 dB threshold produces false SSB confirmations on strong NFM
carriers. 15 dB clears the noise floor while remaining well below the 25–40 dB
asymmetry of a real SSB transmission.

The envelope variance threshold (0.15, normalized) follows similar reasoning:
FM carriers show near-zero variance, AM carriers show variance proportional to
modulation depth. 0.15 sits between the two distributions with margin for
hardware-induced amplitude noise.

Both thresholds were set analytically from synthetic IQ and validated against
the FM broadcast and aviation bands. They may require adjustment for hardware
with higher I/Q imbalance or at the edges of the tuner's range.

### Next analytical steps

The three natural extensions to the current classifier, in priority order:

1. **Per-peak dwell** — the current classifier runs on the full 2 MHz spectrum and
   measures the dominant signal's bandwidth. Running a second shorter-dwell pass
   centered on each detected peak would enable multi-signal discrimination within
   the same view.

2. **Protocol decoder integration** — APRS (144.800 MHz) and AIS (161.975/162.025 MHz)
   have fixed frequencies and known modulations. A lightweight correlation against
   the known symbol rate (1200 baud AFSK, 9600 baud GMSK) would upgrade the
   classification from "this is NFM" to "this is APRS traffic" with no structural
   changes to the classifier architecture.

3. **TDOA with multiple receivers** — with two RTL-SDR dongles and a shared clock
   reference, time-difference-of-arrival provides a bearing line for any classified
   signal. The existing classifier output (frequency, mode, SNR, timestamp) is
   already the correct input format for a TDOA correlator.
