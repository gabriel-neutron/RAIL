# TIMELINE.md — Development Phases and Milestones

> Task-ordered, not time-boxed. Each phase must be fully working and committed before starting the next.

## Table of contents
1. [Phases 0–12 — Completed](#phases-012--completed-)
2. [Phase 13 — Documentation and presentation](#phase-13--documentation-and-presentation)
4. [Phase 14 — UX improvements](#phase-14--ux-improvements)
5. [Phase 15 — Coverage and classifier expansion](#phase-15--coverage-and-classifier-expansion)
6. [Phase 16 — Advanced DSP](#phase-16--advanced-dsp)
7. [Phase 17 — Protocol Decoders](#phase-17--protocol-decoders)
8. [What not to build](#what-not-to-build)

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

## Phase 17 — Protocol Decoders

**Goal**: decode live protocol traffic from real-world signals and surface
structured data in a new Decoder Panel — demonstrating a full RF→protocol
stack in native Rust.

- [ ] **Scaffold decoder module** — `src-tauri/src/decoders/mod.rs`,
      `adsb.rs`, `aprs.rs`, `rds.rs`, `pocsag.rs`
      Module mirrors `dsp/demod/`: one file per protocol; `mod.rs` re-exports only.
      Add `pub mod decoders;` to `lib.rs`. Create `docs/DECODERS.md` and register
      in `docs/README.md`.
      *(1 h)*

- [ ] **Add `DecoderFrame` typed event infrastructure** —
      `src-tauri/src/ipc/events.rs`, `src-tauri/src/ipc/dsp_task.rs`
      Four new event structs: `AdsB1090Frame`, `AprsPacket`, `RdsGroup`,
      `PocsagMessage`; each follows `impl X { pub fn emit<R>(...) }` pattern.
      Add `last_decoder_emit: Instant` per-decoder to `DspTaskCtx`.
      Call `emit_decoder_frames()` in `DspTaskCtx::run()` after `chain.process()`.
      Mirror all four types in `src/ipc/events.ts`.
      *(3–4 h)*

- [ ] **Implement ADS-B 1090 decoder** — `src-tauri/src/decoders/adsb.rs`
      Pulse detector on `|IQ|` magnitude; preamble sync (8-pulse pattern at
      1 Mbit/s timing at 2.4 MHz sample rate); Mode S short (56-bit) and long
      (112-bit) frame extractor; CRC-24 (ICAO Doc 9684 polynomial);
      DF17 extended squitter: ICAO address, type 9–18 (position), type 19
      (velocity), type 4 (callsign).
      Frequency-prior gate: center within 500 kHz of 1_090_000_000 Hz.
      Unit tests: known-good Mode S bytes → assert CRC pass + field parse.
      *(16–24 h)*

- [ ] **Implement APRS / Bell 202 decoder** — `src-tauri/src/decoders/aprs.rs`
      Input: NFM discriminator output (already in `DemodChain::process`);
      downsample to 22.05 kHz; Bell 202 soft-decision correlator (mark=1200 Hz,
      space=2200 Hz); NRZI decode + bit-unstuffing; HDLC flag detection (0x7E);
      AX.25 frame parse; APRS info field parse (position, object, message,
      status, weather report types).
      Frequency-prior gate: within 10 kHz of 144_390_000 or 144_800_000 Hz.
      Unit tests: Bell 202 audio bytes → assert AX.25 extraction.
      *(20–30 h)*

- [ ] **Implement RDS subcarrier decoder** — `src-tauri/src/decoders/rds.rs`
      Input: WBFM baseband at 256 kHz (pre-deemphasis);
      57 kHz pilot extraction via 2× 19 kHz pilot quadrature correlation;
      BPSK symbol recovery at 1187.5 Bd; differential decode;
      26-bit block + 10-bit checkword (RDS standard Annex B);
      Group decode: 0A/0B (PS name), 2A/2B (RadioText), 4A (clock), 14B (EON).
      Frequency-prior gate: center in 87_500_000–108_000_000 Hz AND mode = FM.
      Unit tests: synthetic 57 kHz BPSK group → assert block decode.
      *(16–20 h)*

- [ ] **Implement POCSAG decoder** — `src-tauri/src/decoders/pocsag.rs`
      Input: NFM discriminator output; FSK slicer (zero-crossing bit clock);
      sync codeword detection (0x7CD215D8); BCH(31,21) single-bit error
      correction (poly x^10+x^9+x^8+x^6+x^5+x^3+1); message frame assembly
      (CAPCODE 21-bit + function bits + BCD/ASCII content); baud auto-detect
      (512/1200/2400).
      Frequency-prior gate: center in 152–159 MHz or 929–931 MHz range.
      Unit tests: known POCSAG frame bytes with BCH errors → assert correction.
      *(12–16 h)*

- [ ] **Decoder Panel React component** — `src/components/DecoderPanel/index.tsx`,
      `src/store/decoders.ts`
      `decoders.ts`: zustand store with `frames: DecoderFrame[]` (ring cap 500),
      `visible: boolean`, `toggleVisible()`, `pushFrame(f)`, `clear()`.
      `DecoderFrame`: discriminated union `{ kind: 'adsb', ... } | ...`.
      Panel: scrollable list newest-first; collapsed row = protocol badge +
      timestamp + identifier + key field; click-to-expand for all fields +
      "Copy as JSON" button.
      Subscribe all four events in `App.tsx` (same pattern as
      `subscribeSignalClassification`).
      *(8–12 h)*

- [ ] **Wire show/hide into MenuBar** — `src/components/MenuBar/index.tsx`
      Add `"decoders"` to `MenuKey` union. Add View → "Show/Hide Decoders"
      item, directly below the existing Scanner toggle.
      *(1 h)*

- [ ] **Update SIGNALS.md decoder coverage rows** — `docs/SIGNALS.md`
      Update §4.1 (RDS), §4.5 (APRS), §4.8 (POCSAG), §4.14 (ADS-B)
      "RAIL" column: "not planned" → "Phase 17".
      Add §5.6 "Implemented decoders" table; remove same four from §5.5.
      *(1 h)*

- [ ] **Integration tests: all four decoders via SigMF replay** —
      `tests/fixtures/`, `src-tauri/src/decoders/*.rs #[cfg(test)]`
      Capture short SigMF clips per protocol during development.
      Each test: load clip via `DspInput::Cf32Shifted`, run decoder, assert
      ≥1 valid decoded frame.
      *(4–8 h)*

**Total estimate**: 82–116 h (10–15 working days)

**Exit criterion**: With a live RTL-SDR dongle — tuning to 1090 MHz shows
ADS-B aircraft frames in the Decoder Panel within 30 seconds; tuning to
144.390 MHz shows APRS packets (where local traffic exists); tuning to any
strong FM station shows RDS PS name within 5 seconds; tuning to a POCSAG
frequency shows decoded messages. All four decoders have `#[cfg(test)]` unit
tests passing under `cargo test`. `clippy` clean. TypeScript `any` free.

**What not to build in this phase**
- No aircraft or ship map rendering — frames go to the panel list; map
  visualization is Phase 18. The data model is correct for a future map; the
  renderer is not.
- No AIS in Phase 17 — dual-channel GMSK adds tuning strategy complexity;
  visual impact equivalent to APRS but higher implementation cost. Phase 18.
- No rtl_433 sub-protocol library — 200+ sub-protocols are a maintenance
  trap; curated 5-sensor subset is Phase 18 work.
- No FLEX in Phase 17 — 4-FSK complexity delta not justified over POCSAG for
  portfolio purposes. Phase 18.
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
- **No AIS decoder before Phase 18** — dual-channel GMSK requires a dedicated tuning strategy; save for Phase 18 alongside the map rendering layer.
- **No rtl_433 sub-protocol library before Phase 18** — curating 5–10 sensors is Phase 18 scope; the full 200+ sub-protocol library is a permanent maintenance trap.
- **No FLEX Phase 2/3 in Phase 17** — the FLEX specification is not open; Phase 2/3 4-FSK adds risk with marginal portfolio delta over POCSAG. Phase 18 if at all.
- **No ACARS in Phase 17** — AM-MSK post-demod adds implementation cost; aviation theme is already served by ADS-B. Phase 18.
- **No voice codec integration ever** — P25, DMR, D-STAR require AMBE vocoder. AMBE is patent-encumbered by DVSI. Permanently out of scope.
