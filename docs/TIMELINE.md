# TIMELINE.md — Development Phases and Milestones

> Task-ordered, not time-boxed. Each phase must be fully working and committed before starting the next.

## Table of contents
1. [Phases 0–16 — Completed](#phases-016--completed-)
2. [Phase 13 — Documentation and presentation](#phase-13--documentation-and-presentation)
3. [Phase 14 — UX improvements](#phase-14--ux-improvements)
4. [Phase 15 — Coverage and classifier expansion](#phase-15--coverage-and-classifier-expansion)
5. [Phase 16 — Advanced DSP](#phase-16--advanced-dsp)
6. [Phase 17 — FM demod first: RDS subcarrier](#phase-17--fm-demod-first-rds-subcarrier)
7. [Phase 18 — Decoder foundation](#phase-18--decoder-foundation)
8. [Phase 19 — ADS-B 1090](#phase-19--ads-b-1090)
9. [Phase 20 — APRS / Bell 202](#phase-20--aprs--bell-202)
10. [Phase 21 — POCSAG](#phase-21--pocsag)
11. [Phase 22 — Integration hardening](#phase-22--integration-hardening)
12. [What not to build](#what-not-to-build)

---

## Phases 0–16 — Completed ✓

| Phase | Summary |
|---|---|
| 0 | Project scaffold and prerequisites |
| 1 | IQ stream and waterfall |
| 2 | Frequency control and tuning |
| 3 | Demodulation and audio |
| 4 | Signal meter and UI polish |
| 5 | Capture and session system |
| 6 | V1 hardening |
| 7 | Code compliance and first release |
| 8 | Demodulation expansion (NFM, USB, LSB, CW) |
| 9 | Wideband scanner |
| 10 | Signal intelligence layer |
| 11 | Polish and guided navigation |
| 12 | DSP correctness — FFT window coherent gain fix, 0 dBFS normalization test |
| 13 | Documentation and presentation — README rewrite, SIGNALS.md §6, v0.2.0 tag, field result |
| 14 | UX improvements — squelch control, richer bookmarks, DC offset visualization |
| 15 | Coverage and classifier expansion — FM tests, frequency prior expansion, scanner dwell fix |
| 16 | Advanced DSP |

---

## Phase 17 — FM demod first: RDS subcarrier

- [ ] **Implement RDS subcarrier decoder** — `src-tauri/src/decoders/rds.rs`
      Input: WBFM baseband at 256 kHz (pre-deemphasis);
      57 kHz pilot extraction via 2× 19 kHz pilot quadrature correlation;
      BPSK symbol recovery at 1187.5 Bd; differential decode;
      26-bit block + 10-bit checkword (RDS standard Annex B);
      Group decode: 0A/0B (PS name), 2A/2B (RadioText), 4A (clock), 14B (EON).
      Frequency-prior gate: center in 87_500_000–108_000_000 Hz AND mode = FM.
      Unit tests: synthetic 57 kHz BPSK group → assert block decode.
      *(16–20 h)*

- [ ] **Emit RDS groups to the Decoder Panel**
      Add `RdsGroup` event fields and display PS name / RadioText as the key
      collapsed-row fields when present.
      *(2–3 h)*

**Exit criterion**: Tuning to a strong FM station shows RDS PS name within
5 seconds. RDS tests pass under `cargo test`.

---

## Phase 18 — Decoder foundation

**Goal**: establish the shared decoder architecture and UI surface so each
protocol can be added independently.

- [ ] **Scaffold decoder module** — `src-tauri/src/decoders/mod.rs`
      Module mirrors `dsp/demod/`: one file per protocol; `mod.rs` re-exports only.
      Add `pub mod decoders;` to `lib.rs`.
      *(1 h)*

- [ ] **Add shared decoder event infrastructure** —
      `src-tauri/src/ipc/events.rs`, `src-tauri/src/ipc/dsp_task.rs`
      Add the shared `DecoderFrame` dispatch path, per-decoder emit throttling,
      and TypeScript event mirrors in `src/ipc/events.ts`.
      *(3–4 h)*

- [ ] **Create Decoder Panel shell** — `src/components/DecoderPanel/index.tsx`,
      `src/store/decoders.ts`
      `decoders.ts`: zustand store with `frames: DecoderFrame[]` (ring cap 500),
      `visible: boolean`, `toggleVisible()`, `pushFrame(f)`, `clear()`.
      Panel: scrollable list newest-first; collapsed row = protocol badge +
      timestamp + identifier + key field; click-to-expand for all fields +
      "Copy as JSON" button.
      *(6–8 h)*

- [ ] **Wire show/hide into MenuBar** — `src/components/MenuBar/index.tsx`
      Add `"decoders"` to `MenuKey` union. Add View → "Show/Hide Decoders"
      item, directly below the existing Scanner toggle.
      *(1 h)*

**Exit criterion**: The app can receive a typed decoder frame from Rust, store it
in the frontend ring buffer, show it in the Decoder Panel, clear it, and toggle
the panel from the menu. No protocol-specific parser is required yet.

---

## Phase 19 — ADS-B 1090

- [ ] **Implement ADS-B 1090 decoder** — `src-tauri/src/decoders/adsb.rs`
      Pulse detector on `|IQ|` magnitude; preamble sync (8-pulse pattern at
      1 Mbit/s timing at 2.4 MHz sample rate); Mode S short (56-bit) and long
      (112-bit) frame extractor; CRC-24 (ICAO Doc 9684 polynomial);
      DF17 extended squitter: ICAO address, type 9–18 (position), type 19
      (velocity), type 4 (callsign).
      Frequency-prior gate: center within 500 kHz of 1_090_000_000 Hz.
      Unit tests: known-good Mode S bytes → assert CRC pass + field parse.
      *(16–24 h)*

- [ ] **Emit ADS-B frames to the Decoder Panel**
      Add `AdsB1090Frame` event fields and subscribe in `App.tsx` using the
      same pattern as `subscribeSignalClassification`.
      *(2–3 h)*

**Exit criterion**: With a live RTL-SDR dongle, tuning to 1090 MHz shows ADS-B
aircraft frames in the Decoder Panel within 30 seconds. ADS-B tests pass under
`cargo test`.

---

## Phase 20 — APRS / Bell 202

- [ ] **Implement APRS / Bell 202 decoder** — `src-tauri/src/decoders/aprs.rs`
      Input: NFM discriminator output (already in `DemodChain::process`);
      downsample to 22.05 kHz; Bell 202 soft-decision correlator (mark=1200 Hz,
      space=2200 Hz); NRZI decode + bit-unstuffing; HDLC flag detection (0x7E);
      AX.25 frame parse; APRS info field parse (position, object, message,
      status, weather report types).
      Frequency-prior gate: within 10 kHz of 144_390_000 or 144_800_000 Hz.
      Unit tests: Bell 202 audio bytes → assert AX.25 extraction.
      *(20–30 h)*

- [ ] **Emit APRS packets to the Decoder Panel**
      Add `AprsPacket` event fields and reuse the existing decoder store path.
      *(2–3 h)*

**Exit criterion**: Tuning to 144.390 MHz or 144.800 MHz shows APRS packets
where local traffic exists. APRS tests pass under `cargo test`.

---

## Phase 21 — POCSAG

- [ ] **Implement POCSAG decoder** — `src-tauri/src/decoders/pocsag.rs`
      Input: NFM discriminator output; FSK slicer (zero-crossing bit clock);
      sync codeword detection (0x7CD215D8); BCH(31,21) single-bit error
      correction (poly x^10+x^9+x^8+x^6+x^5+x^3+1); message frame assembly
      (CAPCODE 21-bit + function bits + BCD/ASCII content); baud auto-detect
      (512/1200/2400).
      Frequency-prior gate: center in 152–159 MHz or 929–931 MHz range.
      Unit tests: known POCSAG frame bytes with BCH errors → assert correction.
      *(12–16 h)*

- [ ] **Emit POCSAG messages to the Decoder Panel**
      Add `PocsagMessage` event fields and display CAPCODE + message preview in
      collapsed rows.
      *(2–3 h)*

**Exit criterion**: Tuning to a POCSAG frequency shows decoded messages.
POCSAG tests pass under `cargo test`.

---

## Phase 22 — Integration hardening

- [ ] **Update SIGNALS.md decoder coverage rows** — `docs/SIGNALS.md`
      Update §4.1 (RDS), §4.5 (APRS), §4.8 (POCSAG), §4.14 (ADS-B)
      "RAIL" column: "not planned" → planned phase number.
      Add §5.6 "Implemented decoders" table; remove same four from §5.5.
      *(1 h)*

- [ ] **Integration tests: implemented decoders via SigMF replay** —
      `tests/fixtures/`, `src-tauri/src/decoders/*.rs #[cfg(test)]`
      Capture short SigMF clips per protocol during development.
      Each test: load clip via `DspInput::Cf32Shifted`, run decoder, assert
      ≥1 valid decoded frame.
      *(4–8 h)*

**Total estimate**: 95–125 h (12–16 working days)

**Exit criterion**: All implemented protocol decoders have focused unit tests,
SigMF replay coverage where fixtures are available, `cargo test` passing,
`clippy` clean, and TypeScript `any` free.

**What not to build in these phases**
- No aircraft or ship map rendering — frames go to the panel list.
- No AIS — dual-channel GMSK adds tuning strategy complexity with limited value
  for this decoder sequence.
- No rtl_433 sub-protocol library — 200+ sub-protocols are a maintenance trap.
- No FLEX — 4-FSK complexity delta is not justified over POCSAG for portfolio
  purposes.
- No voice codec integration (P25/DMR/D-STAR) — AMBE patent. Permanently out.
- Do not extend `DemodMode` enum for decoders — the decoder path is a
  side-chain, not a mode replacement.
- Do not render raw hex bytes in the collapsed panel row — experts use expand.

---

## What not to build

- **No `AtomicU64` for center frequency** — RTL-SDR tops at ~1.7 GHz; width is fine. Fix `Relaxed` ordering via coordinated channel message if needed.
- **No third-party UI component library** — the custom UI is a demonstrable technical skill; adding Material-UI erases that signal.
- **No AI-based signal classification in v1** — the heuristic classifier is the right scope; a Burn/TFLite model expands scope without proving more competence.
- **No IPC framing header as P0** — Tauri v2 channel semantics make practical coalescing unlikely; revisit only if frame corruption is observed.
- **Do not expand internal docs** — ARCHITECTURE.md and DSP.md are already unusually thorough; field results and a rewritten README move the portfolio needle more.
- **No AIS decoder in the decoder sequence** — dual-channel GMSK requires a dedicated tuning strategy that is outside current scope.
- **No rtl_433 sub-protocol library** — curating 5–10 sensors can be planned separately; the full 200+ sub-protocol library is a permanent maintenance trap.
- **No FLEX Phase 2/3 in the decoder sequence** — the FLEX specification is not open; Phase 2/3 4-FSK adds risk with marginal portfolio delta over POCSAG.
- **No ACARS in the decoder sequence** — AM-MSK post-demod adds implementation cost; aviation theme is already served by ADS-B.
- **No voice codec integration ever** — P25, DMR, D-STAR require AMBE vocoder. AMBE is patent-encumbered by DVSI. Permanently out of scope.
