# TIMELINE.md — Development Phases and Milestones

> Task-ordered, not time-boxed. Each phase must be fully working and committed before starting the next.

## Table of contents
1. [Phases 0–12 — Completed](#phases-012--completed-)
2. [Phase 13 — Documentation and presentation](#phase-13--documentation-and-presentation)
4. [Phase 14 — UX improvements](#phase-14--ux-improvements)
5. [Phase 15 — Coverage and classifier expansion](#phase-15--coverage-and-classifier-expansion)
6. [Phase 16 — Advanced DSP](#phase-16--advanced-dsp)
7. [What not to build](#what-not-to-build)

---

## Phases 0–12 — Completed ✓

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

---

## Phase 13 — Documentation and presentation

This phase is based on `@REVIEW.md`, read it for extra details.

**Goal**: close the presentation gap — recruiter and hobbyist can evaluate the project without building it.

- [ ] **Rewrite README opening** — `README.md`
      Drop "educational build"; lead with capabilities + tech proof points (hand-written librtlsdr FFI, Hilbert FIR, three-path classifier, SigMF capture); screenshot above the fold; add one-paragraph "Intelligence value" section
      *(1 h)*
- [ ] **Add SIGNALS.md §6 — Classifier design note** — `docs/SIGNALS.md`
      200–300 words: why frequency prior dominates, why `asym=15 dB`, next analytical steps (per-peak dwell, protocol decoder integration, TDOA with multiple receivers)
      *(1–2 h)*
- [ ] **Verify or publish GitHub release** — git + `release.yml`
      Confirm tag `v0.1.0` exists and `.exe` installer is downloadable; if missing, `git tag v0.1.0 && git push origin v0.1.0` and confirm CI build succeeds
      *(30 min–1 h)*
- [ ] **Add field-validation screenshots** — `docs/assets/field/`, `README.md` *(requires hardware session)*
      Capture classifier badge on: FM broadcast (WBFM), ATC/AM, maritime VHF (NFM); add "Field results" section to README
      *(2–4 h)*

**Exit criterion**: README leads with capabilities; installer is downloadable; SIGNALS.md §6 exists; at least one field screenshot is in the repo.

---

## Phase 14 — UX improvements

This phase is based on `@REVIEW.md`, read it for extra details.

**Goal**: surface backend features that are fully implemented but missing from the UI.

- [ ] **Add squelch slider** — `App.tsx` + new `SquelchControl.tsx` component
      Range: −100 to 0 dBFS, "disabled" position at minimum; calls `setSquelchDbfs` store action
      Position: between AudioControls and PpmControl
      *(2 h)*
- [ ] **Extend bookmarks to store mode + bandwidth** — `bookmarks.rs`, `ipc/commands.rs`, `store/bookmarks.ts`
      Add optional `mode: Option<String>` and `bandwidth_hz: Option<u32>` to `Bookmark`; apply on tune (missing fields = no change)
      *(3 h)*
- [ ] **Add DC offset annotation to waterfall** — `Waterfall.tsx`, `FrequencyAxis.tsx`
      Status bar: `DC: ±{sampleRate/4} MHz`; thin annotation line on `FrequencyAxis` at `center_hz ± sample_rate/4`
      *(1 h)*

**Exit criterion**: squelch slider visible and functional on NFM; tuning a bookmark restores mode and bandwidth; waterfall status bar shows DC offset.

---

## Phase 15 — Coverage and classifier expansion

This phase is based on `@REVIEW.md`, read it for extra details.

**Goal**: close test coverage gaps and expand the frequency prior to cover common scan targets.

- [ ] **Add FM demodulator unit test** — `dsp/demod/fm.rs` `#[cfg(test)]`
      Constant-phase-deviation IQ → assert audio amplitude within expected range; cite DSP.md §4
      *(2 h)*
- [ ] **Expand classifier frequency prior** — `classifier.rs` match arm
      Add: NOAA weather radio (162.4–162.55 MHz), FRS/GMRS (462–467 MHz), MURS (151–154 MHz), public safety UHF (450–470 MHz), ACARS (129.125 MHz), DAB III (174–240 MHz)
      *(2 h)*
- [ ] **Fix scanner measurement** — `scanner.rs`, `ipc/dsp_task.rs`
      Replace single `latest_dbfs_bits` poll with float array accumulating `max_dbfs_per_bin` over the full dwell window; enables burst detection and richer band-activity canvas
      *(4–6 h)*

**Exit criterion**: FM demod test passes; common bands return correct classifier labels; scanner catches sub-50 ms burst traffic.

---

## Phase 16 — Advanced DSP

This phase is based on `@REVIEW.md`, read it for extra details.

**Goal**: fix DSP correctness issues causing audible degradation on WBFM and SSB.

- [ ] **DC-blocking IIR before SSB demodulator** — `dsp/demod/ssb.rs`
      2-pole high-pass at ~10 Hz on complex baseband after decimation; eliminates I/Q DC bias before Hilbert phasing; see DSP.md §6
      *(2–3 h)*
- [ ] **Compensate audio LPF group delay in SSB** — `dsp/demod/ssb.rs`
      The 65-tap LPF adds ~32 samples of delay to one path; delay-match the other path to eliminate harmonic distortion
      *(2–4 h)*
- [ ] **Multi-stage polyphase decimation for WBFM** — `dsp/filter.rs`, `dsp/demod/mod.rs`
      Replace single 65-tap 8× decimator with 2-stage (2×4); first-stage filter ≥240 taps for −40 dB stopband at fold-over frequency (128 kHz)
      *(8–12 h)*

**Exit criterion**: WBFM audio free of aliasing artifacts; SSB test tone free of harmonic distortion; `cargo test` green.

---

## What not to build

- **No `AtomicU64` for center frequency** — RTL-SDR tops at ~1.7 GHz; width is fine. Fix `Relaxed` ordering via coordinated channel message if needed.
- **No third-party UI component library** — the custom UI is a demonstrable technical skill; adding Material-UI erases that signal.
- **No AI-based signal classification in v1** — the heuristic classifier is the right scope; a Burn/TFLite model expands scope without proving more competence.
- **No IPC framing header as P0** — Tauri v2 channel semantics make practical coalescing unlikely; revisit only if frame corruption is observed.
- **Do not expand internal docs** — ARCHITECTURE.md and DSP.md are already unusually thorough; field results and a rewritten README move the portfolio needle more.
