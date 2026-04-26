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
as the waterfall and audio. Symbol rates for APRS, RDS and POCSAG (≈500–2400 Bd)
are orders of magnitude below the 2.048 MHz IQ rate, and the ADS-B path at 2.4 Msps
is still lightweight enough to live on the same thread.[cite:2][cite:42]

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
and 2.4 MHz minimum sample rate are required.[cite:11]

**Physical layer**: operate on `|IQ|` magnitude; noise floor = rolling mean over 1000 samples;
pulse threshold = 2× noise floor; preamble = 8-pulse pattern at 1 Mbit/s timing (1 µs pulse
spacing).[cite:42]

**Framing**: short frame 56 bits, long frame (DF17) 112 bits.[cite:42][cite:61]
CRC-24 polynomial (Mode S / ADS-B):

\[G(x) = 1 + x^3 + x^{10} + x^{12} + x^{13} + x^{14} + x^{15} + x^{16} + x^{17} + x^{18} + x^{19} + x^{20} + x^{21} + x^{22} + x^{23} + x^{24}.\] [1]

Implementation constant: `0x1FFF409` including the top bit, or `0xFFF409` with the
implicit \(x^{24}\) term.[cite:2][cite:57]

**DF17 fields**: ICAO 24-bit address (all frames); callsign TC=1–4; airborne position
TC=9–18 (CPR-encoded lat/lon — requires even + odd pair to decode); velocity + heading
TC=19.[cite:42][cite:61]

**Emitted event**: `adsb-1090-frame` (see ARCHITECTURE.md §3.2)

**External references**:
- Mode S / ADS-B CRC and framing: *The 1090 MHz Riddle* (Sun), Eurocontrol / ICAO docs.[cite:2][cite:3][cite:57]
- Example implementation: `adsb-rx` Rust decoder (dump1090 port).[cite:11]

---

## 4. APRS (144 MHz)

**Key reuse**: the NFM demodulator in `dsp/demod/fm.rs` already produces the discriminator
output. The APRS decoder consumes this audio — no separate IQ processing needed.

**Bell 202 modem**: mark = 1200 Hz, space = 2200 Hz, 1200 Bd AFSK on 2 m FM.
Downsample NFM audio (256 kHz) to a standard audio-rate stream (e.g. 22.05 kHz);
apply a soft-decision matched filter over an ≈18-sample window per symbol and take
log-likelihood differences.[cite:54][cite:59]

**NRZI + HDLC**: bit = 1 if no transition, 0 if transition. Remove bit-stuffing by
dropping the `0` that follows any sequence of five consecutive `1`s in the received
bitstream. HDLC flag = `0x7E`; treat ≥7 consecutive `1`s as abort.[cite:24]

**AX.25**: destination + source + 0–8 digipeater addresses; CRC-16-CCITT (FCS) with
polynomial \(G(x) = x^{16} + x^{12} + x^5 + 1\), reflected bit order and AX.25
initial/final values.[cite:29][cite:60] Frames with FCS mismatch are discarded — this
is the main false-positive gate.

**APRS info field**: parse position (`!`/`=`), object (`;`), status (`>`), message (`:`),
weather (`@`/`_`). Unknown types: pass raw string.

**Emitted event**: `aprs-packet` (see ARCHITECTURE.md §3.2)

**External references**:
- APRS over AX.25 using 1200 bit/s Bell 202 AFSK on 2 m band.[cite:54]
- AX.25 v2.2 spec and HDLC framing.[cite:24][cite:29]
- AX.25 CRC details (CRC-16-CCITT).[cite:60]

---

## 5. RDS subcarrier (87.5–108 MHz)

**Key reuse**: WBFM demodulation already runs when mode = FM. The RDS decoder taps
the pre-deemphasis WBFM baseband at 256 kHz.

**57 kHz subcarrier**: `57 kHz = 3 × 19 kHz` stereo pilot. Track pilot phase via
quadrature correlation over a short window; triple the angle to reconstruct the
57 kHz carrier. Perform BPSK demodulation and integrate over one symbol period
(≈ 215.5 samples at 256 kHz) with symbol timing recovered around 1187.5 Bd.[cite:8][cite:15]

**RDS framing**: 1187.5 Bd differential BPSK; 26-bit block (16 data + 10 checkword).
CRC polynomial (RDS standard Annex B):

\[G(x) = x^{10} + x^8 + x^7 + x^5 + x^4 + x^3 + 1\] [2]

which corresponds to `0x5B9`. Four checkword offsets A/B/C/D identify block position.[cite:8][cite:10]

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

**External references**:
- RDS bit rate, block size and CRC polynomial.[cite:8][cite:10][cite:15]
- General RDS group structure and PS/RT/CT/EON fields.[cite:15]

---

## 6. POCSAG (152–159 / 929–931 MHz)

**Key reuse**: NFM discriminator output from `dsp/demod/fm.rs` is the input.

**FSK slicer**: ±4.5 kHz deviation; bit clock recovered from discriminator waveform
(e.g. zero-crossings or timing loop); baud auto-detect between 512, 1200 and 2400 Bd.
Positive discriminator is mapped to one binary level and negative to the other; depending
on RF chain polarity, the mapping may be inverted.[cite:23][cite:28]

**Framing**: sync codeword `0x7CD215D8`; batch = 1 sync + 8 frames × 2 codewords.
Each codeword: 1 flag bit + 20 data bits + 10 BCH parity bits + 1 even parity bit.[cite:17][cite:28]

**BCH(31,21)**: generator polynomial

\[G(x) = x^{10} + x^9 + x^8 + x^6 + x^5 + x^3 + 1\] [3]

(`0x769`), the standard POCSAG BCH(31,21) code. This construction can correct up to
two bit errors per 31-bit codeword; uncorrectable words are marked invalid and the
decoder continues with remaining codewords.[cite:7][cite:12][cite:55]

**Message**: address codeword (flag = 0) carries CAPCODE + function bits
(tone/numeric/alphanumeric); data codewords (flag = 1) carry BCD numeric or 7-bit
ASCII payload.[cite:17][cite:28]

**Emitted event**: `pocsag-message` (see ARCHITECTURE.md §3.2)

**Privacy note**: POCSAG messages may contain medical or emergency PII. The Decoder
Panel shows CAPCODE only in collapsed view; message content requires explicit expand.

**External references**:
- POCSAG framing, sync word, baud rates, deviation.[cite:17][cite:20][cite:23][cite:28]
- BCH(31,21)+parity as used in POCSAG.[cite:7][cite:12][cite:55]

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
- ADS-B: known-good Mode S frame bytes (from public FAA/ICAO / Mode S test vectors) → assert CRC pass + all DF17 fields correct[cite:2][cite:42]
- APRS: recorded Bell 202 audio bytes → assert AX.25 FCS pass + APRS info field parse[cite:24][cite:29]
- RDS: synthetic 57 kHz BPSK block → assert block CRC pass + group type decode[cite:8][cite:15]
- POCSAG: known POCSAG codewords with injected BCH errors → assert correction + message decode[cite:7][cite:12][cite:17]

**Integration tests** (SigMF replay):
Short `.sigmf-data` clips captured per protocol live in `tests/fixtures/`.
Each test loads a clip via `DspInput::Cf32Shifted` (same path as the replay system),
runs the DSP task, and asserts that at least one valid decoded frame is emitted.

SigMF clips for APRS and POCSAG can be sourced from community archives
(e.g. https://www.sigidwiki.com) if live capture is not possible during development.
