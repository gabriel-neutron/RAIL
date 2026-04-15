# PRD — RAIL: Radio Analysis and Intel Lab

## 1. Product summary

RAIL is a Tauri desktop application for RTL-SDR reception and signal analysis.
It targets users ranging from RF beginners to SIGINT professionals, offering
a clean modern interface without sacrificing technical depth.

**Core value proposition**: the first RTL-SDR tool with a professional UI,
a structured capture workflow, and an analysis pipeline designed for
signal intelligence — not just casual listening.

---

## 2. Goals

### Primary (V1)
- Local RTL-SDR listening with live waterfall
- FM and AM demodulation with clean audio
- Structured capture: IQ clips, audio, sessions with metadata
- Modern UI that replaces WebSDR-style dense layouts

### Secondary (V2)
- Wideband scanning and band occupancy view
- Automatic channel detection
- Signal classification (modulation family)
- Capture comparison and recurring channel tracking

### Non-goals (explicitly excluded)
- Remote SDR / network SDR hosting
- GNU Radio integration
- Transmit capability
- Mobile support
- Multi-device concurrent use

---

## 3. Target users

| User type | Needs | RAIL serves them by |
|---|---|---|
| RF beginner | Simple tuning, hear something fast | One-click FM, visual waterfall, auto gain |
| Ham operator | Mode control, filter precision, meter | Mode buttons, filter controls, dBm meter |
| Security researcher | Capture workflow, session notes | SigMF export, session annotations |
| SIGINT analyst | Analysis pipeline, comparison | V2 features, structured data model |

---

## 4. V1 feature requirements

### 4.1 Receiver view (main screen)

| Feature | Priority | Notes |
|---|---|---|
| Live waterfall (scrolling spectrogram) | P0 | 25 fps, canvas-rendered |
| Spectrum view (magnitude curve) | P1 | Above waterfall |
| Frequency input + step controls | P0 | Hz/kHz/MHz, keyboard support |
| Click-to-tune on waterfall | P0 | Maps pixel to frequency |
| Mode selector: FM, AM | P0 | USB/LSB in V1.1 |
| Filter bandwidth control | P0 | Presets + manual |
| Volume + mute | P0 | Web Audio API |
| Squelch control | P1 | Threshold in dBm |
| Signal meter (current + peak dBm) | P1 | |
| Gain control (auto / manual dB) | P1 | Discrete steps from hardware |
| Waterfall zoom | P1 | Frequency span adjustment |
| Bookmark system | P2 | Save named frequencies |
| Keyboard shortcuts | P2 | Arrow keys for tuning |
| PPM correction | P2 | Calibration setting |

### 4.2 Capture view

| Feature | Priority | Notes |
|---|---|---|
| Audio recording (WAV) | P0 | One-click start/stop |
| IQ clip capture (SigMF) | P0 | User-defined duration |
| Waterfall screenshot (PNG) | P1 | |
| Session list with metadata | P0 | Label, freq, mode, date |
| Session notes editor | P1 | Free text + signal type tag |
| Export session as ZIP | P2 | SigMF + WAV + PNG + JSON |

### 4.3 Settings

| Feature | Priority | Notes |
|---|---|---|
| Device selector | P0 | If multiple dongles present |
| Sample rate selector | P1 | From validated list |
| PPM correction | P1 | |
| Capture output directory | P1 | |
| De-emphasis region (50/75µs) | P2 | EU vs US FM |

---

## 5. Technical requirements

| Requirement | Specification |
|---|---|
| Platform | Tauri v2 desktop (Linux, macOS, Windows) |
| Backend | Rust (stable) |
| Frontend | React 18 + TypeScript |
| Hardware | RTL-SDR via direct librtlsdr FFI |
| FFT | rustfft (no custom FFT) |
| Audio | Web Audio API (PCM from Rust) |
| IPC streaming | Tauri binary events (float32 ArrayBuffer) |
| Capture format | SigMF (IQ), WAV (audio), PNG (waterfall) |
| Minimum sample rate | 225 kHz |
| Maximum stable sample rate | 2.4 MHz |
| Waterfall frame rate | 25 fps target |
| Audio output rate | 44100 Hz |

---

## 6. Architecture principles

- All DSP in Rust. React renders only.
- No business logic in the frontend.
- Binary IPC for all streaming data (not JSON).
- SigMF as the canonical capture format.
- Error handling: hardware errors surface to UI. DSP errors skip frame and log.

Full architecture: see `/docs/ARCHITECTURE.md`.

---

## 7. Quality requirements

- No `clippy` warnings in Rust
- No `any` in TypeScript
- All public Rust functions documented
- All captures readable by external SigMF tools
- App usable without documentation (UI is self-explanatory)
- RTL-SDR disconnect handled without crash

---

## 8. Success criteria

### V1 is successful when
- User opens app, sees device detected
- Tunes to 88–108 MHz FM, hears clear audio within 30 seconds
- Records a session with notes
- Closes and reopens app, finds the session

### V2 is successful when
- User scans a band without manually tuning each frequency
- App highlights candidate active channels
- User compares two captures of the same frequency across time

---

## 9. Out of scope clarifications

- RAIL does not host a WebSDR server
- RAIL does not decode digital protocols (APRS, D-Star, DMR) in V1
- RAIL does not support HF direct sampling in V1
- RAIL does not support multiple simultaneous dongles in V1
