# DECODERS.md — Protocol Decoder Reference

## Table of contents
1. [Architecture](#1-architecture)
2. [Frequency gating](#2-frequency-gating)
3. [ADS-B (1090 MHz)](#3-ads-b-1090-mhz)
4. [APRS (144 MHz)](#4-aprs-144-mhz)
5. [RDS subcarrier (87.5–108 MHz)](#5-rds-subcarrier-875108-mhz)
6. [POCSAG (152–159 / 929–931 MHz)](#6-pocsag-152159--929931-mhz)
7. [Error handling](#7-error-handling)
8. [Testing approach](#8-testing-approach)

---

## 1. Architecture

Decoders are a **side-chain** that runs inside `DspTaskCtx::run()` after the main
demodulation chain (`chain.process()`). They share the same `spawn_blocking` thread
as the waterfall and audio — no separate thread is needed because symbol rates
(1200–9600 Bd) are orders of magnitude below the 2.048 MHz IQ rate.

Each decoder:
- Receives either raw IQ (ADS-B) or the NFM/WBFM discriminator audio output (APRS, RDS, POCSAG)
- Returns `Option<DecodedFrame>` — never panics on corrupt input
- Emits a typed Tauri JSON event (see `ARCHITECTURE.md §3.2` and §5.4)
- Is gated by `center_hz_bits` — active only when the frequency prior matches (§2 below)

Module location: `src-tauri/src/decoders/` (one file per protocol).
Do not put decoder logic inside `dsp/demod/` — the demod chain handles audio extraction;
the decoder module handles protocol framing.

---

## 2. Frequency gating

Decoders activate automatically based on the tuned center frequency. No user toggle.
The Decoder Panel shows "No decoder active at this frequency" when none match.

| Decoder | Active when `center_hz` | Source constraint |
|---|---|---|
| ADS-B | within 500 kHz of 1,090,000,000 Hz | HARDWARE.md §4: sensitivity adequate; 2.4 MHz sample rate required |
| APRS | within 10 kHz of 144,390,000 Hz (US) or 144,800,000 Hz (EU) | SIGNALS.md §4.5 frequency prior |
| RDS | in 87,500,000–108,000,000 Hz AND current mode = FM | SIGNALS.md §4.1; WBFM baseband must be active |
| POCSAG | in 152,000,000–159,000,000 Hz or 929,000,000–931,000,000 Hz | SIGNALS.md §4.8 |

---

## 3. ADS-B (1090 MHz)

**Hardware note** (see HARDWARE.md §4): R820T2 sensitivity is reduced above 900 MHz.
ADS-B is still receivable but range is limited vs. 300–900 MHz. A 1090 MHz whip antenna
and 2.4 MHz minimum sample rate are required.

**Physical layer**: operate on `|IQ|` magnitude; noise floor = rolling mean over 1000 samples; pulse threshold = 2× noise floor; preamble = 8-pulse pattern at 1 Mbit/s timing.

**Framing**: short frame 56 bits, long frame (DF17) 112 bits.
CRC-24 polynomial (ICAO Doc 9684, Appendix B): `G(x) = x^24 + x^23 + x^10 + x^3 + 1` (0xFFF409).

**DF17 fields**: ICAO 24-bit address (all frames); callsign TC=1–4; airborne position TC=9–18 (CPR-encoded lat/lon — requires even + odd pair to decode); velocity + heading TC=19.

**Emitted event**: `adsb-1090-frame` (see ARCHITECTURE.md §3.2)

---

## 4. APRS (144 MHz)

**Key reuse**: the NFM demodulator in `dsp/demod/fm.rs` already produces the discriminator
output. The APRS decoder consumes this audio — no separate IQ processing needed.

**Bell 202 modem**: mark=1200 Hz, space=2200 Hz, 1200 Bd. Downsample NFM audio (256 kHz) to 22.05 kHz; soft-decision matched filter over 18-sample window; take log-likelihood difference.

**NRZI + HDLC**: bit = 1 if no transition, 0 if transition; remove bit-stuffing (drop `1` after five consecutive `1`s); HDLC flag = `0x7E`; abort on seven consecutive `1`s.

**AX.25**: destination + source + 0–8 digipeater addresses; CCITT CRC-16 (FCS); discard on FCS mismatch — this is the false-positive gate.

**APRS info field**: parse position (`!`/`=`), object (`;`), status (`>`), message (`:`), weather (`@`/`_`). Unknown types: pass raw string.

**Emitted event**: `aprs-packet` (see ARCHITECTURE.md §3.2)

---

## 5. RDS subcarrier (87.5–108 MHz)

**Key reuse**: WBFM demodulation already runs when mode = FM. The RDS decoder taps
the pre-deemphasis WBFM baseband at 256 kHz.

**57 kHz subcarrier**: `57 kHz = 3 × 19 kHz` stereo pilot. Track pilot phase via quadrature correlation over 256-sample window; triple the angle to reconstruct 57 kHz carrier; BPSK integrate over one symbol period (≈ 215 samples at 256 kHz).

**RDS framing**: 1187.5 Bd differential BPSK; 26-bit block (16 data + 10 checkword).
CRC polynomial (RDS standard Annex B): `G(x) = x^10 + x^8 + x^7 + x^5 + x^4 + x^3 + 1` (0x5B9). Four checkword offsets A/B/C/D identify block position.

**Group decode (relevant types):**
| Group | Content |
|---|---|
| 0A / 0B | PS (Programme Service) name — 8 chars, 2 per group |
| 2A / 2B | RadioText — up to 64 chars, fragment-assembled |
| 4A | Clock-time (UTC offset, date, time) |
| 14B | EON alternate frequency list |

PS name assembly: buffer `pending_ps: [Option<char>; 8]`; emit only when all 8 slots
are filled, or after a 2-second timeout (partial name acceptable for display).

**Emitted event**: `rds-group` (see ARCHITECTURE.md §3.2)

---

## 6. POCSAG (152–159 / 929–931 MHz)

**Key reuse**: NFM discriminator output from `dsp/demod/fm.rs` is the input.

**FSK slicer**: ±4.5 kHz deviation; bit clock recovered from zero-crossings; baud auto-detect (512, 1200, 2400 Bd); positive discriminator → 1, negative → 0.

**Framing**: sync codeword `0x7CD215D8`; batch = 1 sync + 8 frames × 2 codewords.
Each codeword: 1 flag bit + 20 data bits + 10 BCH parity + 1 even parity.

**BCH(31,21)**: generator poly `x^10 + x^9 + x^8 + x^6 + x^5 + x^3 + 1` (0x769); corrects single-bit errors; on uncorrectable error, mark codeword invalid and continue.

**Message**: address codeword (flag=0) carries CAPCODE + function bits (tone/numeric/alphanumeric); data codewords (flag=1) carry BCD numeric or 7-bit ASCII payload.

**Emitted event**: `pocsag-message` (see ARCHITECTURE.md §3.2)

**Privacy note**: POCSAG messages may contain medical or emergency PII. The Decoder
Panel shows CAPCODE only in collapsed view; message content requires explicit expand.

---

## 7. Error handling

All decoder functions return `Option<DecodedFrame>` or `Result<DecodedFrame, DspError>`.

- **CRC / FCS mismatch**: return `None`; increment drop counter at power-of-two thresholds (same pattern as waterfall frame drops)
- **BCH uncorrectable error** (POCSAG): mark codeword invalid; attempt to continue frame; log `warn!` with codeword hex
- **Bell 202 correlator below threshold**: return `None`; no log (expected on voice traffic near 144 MHz)
- **RDS group incomplete after 2 s**: emit partial PS name with `incomplete: true` flag
- **Fatal decoder init failure**: propagate as `DspError` through `RailError`; displayed as toast; does not stop the stream

DSP task thread never panics on decoder errors. A corrupt preamble or malformed frame
silently drops. The waterfall and audio paths are unaffected.

---

## 8. Testing approach

**Unit tests** (`#[cfg(test)]` in each decoder file):
- ADS-B: known-good Mode S frame bytes (from public FAA/ICAO test vectors) → assert CRC pass + all DF17 fields correct
- APRS: recorded Bell 202 audio bytes → assert AX.25 FCS pass + APRS info field parse
- RDS: synthetic 57 kHz BPSK block → assert block CRC pass + group type decode
- POCSAG: known POCSAG codewords with injected single-bit BCH errors → assert correction + message decode

**Integration tests** (SigMF replay):
Short `.sigmf-data` clips captured per protocol live in `tests/fixtures/`.
Each test loads a clip via `DspInput::Cf32Shifted` (same path as the replay system),
runs the DSP task, and asserts that at least one valid decoded frame is emitted.

SigMF clips for APRS and POCSAG can be sourced from community archives
(e.g. https://www.sigidwiki.com) if live capture is not possible during development.
