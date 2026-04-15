# TIMELINE.md — Development Phases and Milestones

> This timeline is task-ordered, not time-boxed.
> Each phase must be fully working and committed before starting the next.
> Claude Code builds one phase at a time. Do not jump ahead.

---

## Phase 0 — Project scaffold and prerequisites

**Goal**: verified, buildable empty project. Nothing ships until this is green.

- [ ] Tauri v2 project initialized (`create-tauri-app`)
- [ ] Rust toolchain verified (`cargo build` passes)
- [ ] React + TypeScript frontend initialized
- [ ] `rtlsdr-rs` or raw FFI dependency added and compiled
- [ ] RTL-SDR device detected and opened in Rust (log device info)
- [ ] librtlsdr installed and linked on host platform
- [ ] Basic Tauri command (`ping` → `pong`) working end-to-end
- [ ] `rustfft` dependency added
- [ ] `zustand` added to frontend
- [ ] `/docs/` folder structure in place
- [ ] `CLAUDE.md` and all docs committed

**Exit criterion**: `cargo tauri dev` launches, RTL-SDR is detected, ping command works.

---

## Phase 1 — IQ stream and waterfall

**Goal**: live waterfall visible in the UI from real hardware.

- [ ] RTL-SDR async stream running in Rust (raw IQ bytes flowing)
- [ ] IQ conversion: u8 → f32 complex (see HARDWARE.md §1)
- [ ] FFT pipeline: Hann window → rustfft → magnitude → dB → FFT shift (see DSP.md §2–3)
- [ ] Binary Tauri event emitting float32 waterfall frames
- [ ] React canvas component receiving float32 ArrayBuffer
- [ ] Colormap applied (dB → RGB, 6-stop gradient)
- [ ] Waterfall scrolls downward at ~25 fps
- [ ] Frequency display showing current center frequency
- [ ] Gain control (auto/manual) wired to hardware

**Exit criterion**: open app, see live waterfall scrolling with real spectrum data.

---

## Phase 2 — Frequency control and tuning

**Goal**: user can tune to any frequency and see the waterfall update.

- [ ] Frequency input box (numeric, Hz/kHz/MHz toggle)
- [ ] Step buttons (1 Hz, 1 kHz, 10 kHz, 100 kHz steps)
- [ ] Click-to-tune on waterfall canvas (map pixel X → frequency offset)
- [ ] Keyboard shortcuts (arrow keys for step tuning)
- [ ] Bookmark system (save/load named frequencies)
- [ ] DC offset handling (center freq offset — see DSP.md §1)
- [ ] PPM correction setting exposed in UI

**Exit criterion**: click on a signal in the waterfall, frequency updates, waterfall recenters.

---

## Phase 3 — Demodulation and audio

**Goal**: user can listen to FM and AM signals.

- [ ] FM demodulation implemented in Rust (see DSP.md §4)
- [ ] AM demodulation implemented in Rust (see DSP.md §5)
- [ ] Decimation chain to 44100 Hz audio (see DSP.md §4)
- [ ] De-emphasis filter for WBFM (see DSP.md §4)
- [ ] PCM audio streamed via binary Tauri event
- [ ] Web Audio API playback in React (AudioContext)
- [ ] Mode selector buttons: FM / AM (USB/LSB stubbed for V1.1)
- [ ] Volume slider and mute toggle
- [ ] Filter bandwidth control (affects demodulation bandwidth)
- [ ] Squelch control (silence below threshold dBm)

**Exit criterion**: tune to 87.5–108 MHz FM station, hear music clearly.

---

## Phase 4 — Signal meter and UI polish

**Goal**: professional-looking UI with signal strength display.

- [ ] Signal meter: current dBm + peak hold (see DSP.md §2)
- [ ] Waterfall zoom (adjust displayed frequency span)
- [ ] Spectrum view above waterfall (magnitude curve)
- [ ] Filter width visualization on waterfall (shaded region)
- [ ] UI layout finalized: controls panel, waterfall pane, meter
- [ ] Dark theme (SDR tools are always dark)
- [ ] Responsive layout (minimum 1280px width target)
- [ ] Device status indicator (connected / disconnected)
- [ ] Error handling: device disconnect handled gracefully

**Exit criterion**: app looks like a real tool. Screenshot-worthy.

---

## Phase 5 — Capture and session system

**Goal**: user can save and revisit captures.

- [ ] Audio recording (WAV, PCM float32, 44100 Hz)
- [ ] Waterfall screenshot (PNG)
- [ ] IQ clip capture (SigMF format — see SIGNALS.md §1)
- [ ] Session metadata saved (see SIGNALS.md §2)
- [ ] Capture list view (list sessions, show metadata)
- [ ] Session notes editor (label, frequency, mode, signal type, free text)
- [ ] Open/play back audio recording
- [ ] Export session as ZIP (SigMF + WAV + PNG + JSON)

**Exit criterion**: capture a signal, close app, reopen, find the session, play it back.

---

## Phase 6 — V1 hardening and GitHub release

**Goal**: public-ready V1.0 on GitHub.

- [ ] README.md with screenshots and demo GIF
- [ ] Installation instructions for Linux, macOS, Windows
- [ ] Platform-specific driver notes (udev rules, Zadig)
- [ ] `CONTRIBUTING.md` (brief, since solo project)
- [ ] GitHub Actions CI (cargo build + clippy)
- [ ] GitHub release with binary artifacts (Tauri produces installers)
- [ ] All `clippy` warnings resolved
- [ ] No `unwrap()` in non-test code
- [ ] All `/docs/` files reviewed and accurate

**Exit criterion**: a stranger can clone, install, and use the app following the README.

---

## Phase 7 — V2 analysis features (post-V1)

Scope defined after V1 is shipped. Do not plan implementation details now.

- Channel detection (peak finding in spectrum)
- Wideband scanning (sweep + stitch waterfall)
- Signal classification (first-pass modulation labeling)
- Capture comparison view
- Signal annotation and tagging system
