# TIMELINE.md — Development Phases and Milestones

> This timeline is task-ordered, not time-boxed.
> Each phase must be fully working and committed before starting the next.
> Claude Code builds one phase at a time. Do not jump ahead.

## Table of contents
1. [Phase 0 — Project scaffold](#phase-0--project-scaffold-and-prerequisites-)
2. [Phase 1 — IQ stream and waterfall](#phase-1--iq-stream-and-waterfall-)
3. [Phase 2 — Frequency control and tuning](#phase-2--frequency-control-and-tuning-)
4. [Phase 3 — Demodulation and audio](#phase-3--demodulation-and-audio-)
5. [Phase 4 — Signal meter and UI polish](#phase-4--signal-meter-and-ui-polish-)
6. [Phase 5 — Capture and session system](#phase-5--capture-and-session-system-)
7. [Phase 6 — V1 hardening](#phase-6--v1-hardening-)
8. [Phase 7 — Code compliance and first release](#phase-7--code-compliance-and-first-release)
9. [Phase 8 — Demodulation expansion](#phase-8--demodulation-expansion)
10. [Phase 9 — Wideband scanner](#phase-9--wideband-scanner)
11. [Phase 10 — Signal intelligence layer](#phase-10--signal-intelligence-layer)
12. [Phase 11 — Polish and guided navigation](#phase-11--polish-and-guided-navigation)

---

## Phase 0 — Project scaffold and prerequisites ✓

## Phase 1 — IQ stream and waterfall ✓

## Phase 2 — Frequency control and tuning ✓

## Phase 3 — Demodulation and audio ✓

## Phase 4 — Signal meter and UI polish ✓

## Phase 5 — Capture and session system ✓

## Phase 6 — V1 hardening ✓

---

## Phase 7 — Code compliance and first release ✓

**Goal**: close the gap between Phase 6 checklist and actual state; ship v0.1.0 to GitHub.

- [x] Replace all non-test `unwrap()` calls in DSP modules with `.expect("reason")`
      or `?` propagation — files: `dsp/am.rs`, `dsp/fft.rs`, `dsp/filter.rs`, `dsp/waterfall.rs`
- [x] Add table of contents to `docs/TECH_STACK.md` (183 lines, rule: >~150 lines requires TOC)
- [x] Add table of contents to `docs/PRD.md` (155 lines, rule: >~150 lines requires TOC)
- [x] Add quick-start section to `README.md`: prerequisites + install command + run command
- [x] Add `LICENSE` file (MIT)
- [x] `git tag v0.1.0 && git push origin v0.1.0`
      → GitHub Actions `release.yml` builds all installers and publishes the release automatically

**Exit criterion**: GitHub release page exists at `releases/tag/v0.1.0`,
`.exe` installer is downloadable, CI is green on the tag.

---

## Phase 8 — Demodulation expansion

**Goal**: user can demodulate narrow FM, upper and lower sideband signals —
opening aviation repeaters, maritime voice, PMR446, and ham radio SSB.

- [ ] NFM demodulation (narrow FM): FM discriminator with 12.5 kHz filter
      — same FM math as WBFM but with a narrower channel filter; see DSP.md §4
- [ ] De-emphasis for NFM (300–3000 Hz voice shelf, not the 50/75 µs WBFM curve)
- [ ] USB demodulation (upper sideband SSB): analytic signal shift + filter; see DSP.md §4
- [ ] LSB demodulation (lower sideband SSB): mirror of USB
- [ ] Mode selector updated: `WBFM | NFM | AM | USB | LSB`
- [ ] Filter bandwidth presets per mode:
      WBFM → 200 kHz, NFM → 12.5 kHz, AM → 10 kHz, USB/LSB → 3 kHz
- [ ] Squelch threshold recalibrated for NFM noise floor (different from WBFM)

**Exit criterion**: tune to 156.800 MHz (maritime VHF channel 16), select NFM,
hear voice or carrier. Tune to 144.200 MHz (2m SSB calling), select USB, hear SSB voice.

---

## Phase 9 — Wideband scanner

**Goal**: user can sweep a frequency range and auto-discover active signals
without manually stepping through frequencies.

- [ ] Scanner engine in Rust: configurable start freq, stop freq, step size, dwell time
- [ ] Sequential tuning loop: `tune → wait dwell → measure peak power → advance`
- [ ] Scan-stop condition: peak power exceeds squelch threshold during dwell
- [ ] Sweep result emitted as a float32 power-per-step array via binary Tauri event
- [ ] Band activity canvas in React: horizontal bar showing power across scanned range,
      same colormap as waterfall
- [ ] Active-signal markers overlaid on band activity (vertical lines at peaks)
- [ ] User controls: start/stop, step size (default = current BW ≈ 2.4 MHz),
      dwell time (default 200 ms), scan range input
- [ ] Click-to-tune on band activity canvas: clicking a marker tunes the main receiver

**Exit criterion**: scanner sweeps 87–108 MHz in 200 kHz steps,
the band activity canvas fills with power levels, active FM stations appear as peaks,
clicking a peak tunes the receiver and the waterfall updates.

---

## Phase 10 — Signal intelligence layer

> **Prerequisite**: read `docs/SIGNALS.md` §4 (receivable signal reference by band)
> and §5 (classification heuristics) in full before writing any detection or
> classification code. All classifier logic must cite SIGNALS.md §5, not restate it.

**Goal**: the app detects, measures, and classifies signals automatically —
emitting a structured suggestion the UI can display without the user selecting a mode.

- [ ] Peak detector in Rust: find local maxima in FFT magnitude above estimated noise floor
      — see DSP.md §2 for magnitude pipeline
- [ ] Bandwidth estimator: measure –3 dB and –10 dB width around each detected peak
- [ ] Envelope variance measurement: discriminate AM from FM family
      — see SIGNALS.md §5.2 and DSP.md §4–5 for signal math
- [ ] Spectral flatness measurement: discriminate analog from digital signals
      — see SIGNALS.md §5.2
- [ ] Frequency prior lookup: match current center frequency against band table
      in SIGNALS.md §5.3
- [ ] Combine heuristic result + frequency prior into confidence-scored label
- [ ] Output contract: emit `{label, confidence, reason}` per SIGNALS.md §5.4
      as a low-rate JSON Tauri event (not binary)
- [ ] Frontend: "Suggested mode" badge near mode selector
      — shows label and confidence, tooltip shows `reason`
      — badge is display only; does not change the active mode automatically

**Exit criterion**: tune to 98 MHz without selecting a mode — badge reads
`WBFM — high confidence`. Tune to 156.800 MHz — badge reads `NBFM — high confidence`.
Tune to blank noise — badge reads nothing or `unknown`.

---

## Phase 11 — Polish and guided navigation

**Goal**: tie signal intelligence into navigation UX; lower the barrier for
first-time users; complete the portfolio-ready state of the app.

- [ ] Band quick-access shortcuts: clickable entries for FM Broadcast, Aviation,
      Maritime VHF, 2m Amateur, ISM 433, PMR446 — each jump sets center frequency
      and optionally triggers a quick scan (±10 MHz around band center)
- [ ] Suggested mode auto-apply: opt-in toggle in settings — when enabled, the
      classifier output from Phase 10 automatically selects the demodulation mode
      on each retune; disabled by default
- [ ] `signal_type_guess` in session schema auto-populated from classifier output
      at capture time — see SIGNALS.md §2 schema field
- [ ] Waterfall export: PNG with frequency axis, center frequency, timestamp,
      and classifier label burned into the image header area
- [ ] Keyboard shortcut to trigger a quick scan of ±10 MHz around current frequency
- [ ] First-use onboarding hint: one-time overlay pointing to the band shortcuts
      and suggested mode badge

**Exit criterion**: a user who has never used SDR software opens the app,
clicks "FM Broadcast" in the band shortcuts, the app tunes to 98 MHz,
the suggested mode badge shows `WBFM`, and (if auto-apply is on) FM audio starts
— without reading any documentation.
