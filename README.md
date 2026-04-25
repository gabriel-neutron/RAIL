<div align="center">

# RAIL

### *Radio Analysis & Intel Lab*

**RTL-SDR desktop app — live waterfall, multi-mode demodulation, wideband scanner, and automated signal classification.**

<br />

<img src="docs/assets/image.png" alt="RAIL — FM broadcast at 106 MHz, classified as WBFM at 42 dB SNR" width="960" />

*FM broadcast at 106 MHz. Classifier confirms WBFM at 42 dB SNR. Waterfall shows adjacent-channel activity across 2 MHz.*

<br />

[![Rust](https://img.shields.io/badge/Rust-backend-DEA584?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Tauri-desktop-FFC131?style=for-the-badge&logo=tauri&logoColor=000)](https://tauri.app/)
[![React](https://img.shields.io/badge/React-UI-61DAFB?style=for-the-badge&logo=react&logoColor=000)](https://react.dev/)

</div>

---

## What RAIL does

| Capability | Details |
| :--- | :--- |
| **Live waterfall** | Scrolling spectrogram at 25 fps, canvas-rendered with no third-party library |
| **Six demodulation modes** | WBFM, NFM, AM, USB, LSB, CW — all DSP in Rust |
| **Signal meter** | dBFS level + peak hold; 0 dBFS calibrated against a full-scale reference tone |
| **Wideband scanner** | Sweeps a user-defined frequency range, auto-discovers active signals |
| **Signal classifier** | Three-path dispatch: frequency prior → bandwidth → modulation analysis; emits a mode suggestion with SNR and reason |
| **SigMF capture** | IQ clips, audio recordings, waterfall screenshots; sessions with metadata |
| **Offline replay** | Full pipeline runs against saved IQ files — no hardware needed |
| **Bookmarks** | Named frequencies with one-click tune |
| **PPM correction** | Calibration offset per dongle |

---

## Technical proof points

- **Hand-written librtlsdr FFI** — direct binding via `libloading`, no `rtl_tcp` daemon; works with real hardware and CI-fetched prebuilts on all three platforms
- **From-scratch Hilbert FIR phasing** — 129-tap Hilbert filter with matched group-delay compensation on both I/Q paths; used for USB and LSB demodulation
- **Three-path signal classifier** — frequency prior, bandwidth measurement (6 dB above noise floor), and envelope/asymmetry discrimination; see `docs/SIGNALS.md §5` for the full design
- **Binary IPC** — waterfall frames and audio stream sent as float32 `ArrayBuffer` Tauri events, not JSON; keeps the 25 fps budget without serialization overhead
- **Coherent-gain-correct FFT** — window normalization divides by the sum of window coefficients, not `N`; all dB readings are calibrated to 0 dBFS
- **SigMF-compliant capture** — IQ files readable by any SigMF-aware tool (GNU Radio, SigMF Python library, inspectrum)

---

## Intelligence value

I built RAIL because I wanted to understand what makes a real SIGINT pipeline work — not just call a library and display a plot. The part that required the most deliberate design was the classifier: choosing to put the frequency prior first (rather than running spectral analysis on every frame) was a decision driven by the false-positive problem. When you're monitoring a single known band — FM broadcast, aviation voice, maritime VHF — running envelope variance and sideband asymmetry tests on every emission just introduces cycling. The prior suppresses that. Spectral analysis only runs when the prior returns more than one candidate, which is exactly the case where it adds information (2m amateur: FM voice vs SSB vs CW vs APRS). The asymmetry threshold of 15 dB is higher than you'd set analytically because 4 ms IQ windows over RTL-SDR hardware have enough measurement noise to produce 5–10 dB asymmetry on what is actually a symmetric NFM signal. I documented the threshold choices and the next steps — per-peak dwell, protocol decoder integration, TDOA with multiple receivers — in `docs/SIGNALS.md §6`.

---

## Field result

<img src="docs/assets/image.png" alt="Classifier confirming WBFM on FM broadcast at 106 MHz" width="840" />

FM broadcast at 106.076 MHz. Classifier badge: **WBFM confirmed, 42 dB SNR**. The classifier dispatched through the single-prior path (FM broadcast band → one candidate → trust prior directly) and confirmed based on spectral SNR exceeding the 20 dB threshold. Adjacent stations visible in the waterfall at ±200 kHz spacing as expected for broadcast FM.

---

## Quick start

**Prerequisites**

- [Rust](https://rustup.rs/) stable toolchain
- [Node.js](https://nodejs.org/) 20+
- `librtlsdr` installed for your platform:
  - **Windows** — install [Zadig](https://zadig.akeo.ie/) and replace the dongle driver with WinUSB, then drop the librtlsdr prebuilts into `vendor/librtlsdr-win-x64/` (see [`docs/HARDWARE.md`](docs/HARDWARE.md) §6)
  - **macOS** — `brew install librtlsdr`
  - **Linux** — `sudo apt install librtlsdr-dev`
- An RTL-SDR dongle (or skip to **Offline demo** below if you don't have one)

**Install and run**

```bash
npm install
npm run tauri dev
```

**Download the installer** — see [Releases](../../releases) for the Windows `.exe`, macOS `.dmg`, and Linux `.AppImage`.

---

## Offline demo

No dongle? The repo ships with a short IQ capture at [`docs/assets/demo_iq.sigmf-data`](docs/assets/demo_iq.sigmf-data) (101.58 MHz FM, recorded 2026-04-19). Open it from the replay transport — the waterfall, spectrum, and audio paths run end-to-end against the file with no hardware.

---

## Going deeper

Architecture, DSP math, and signal reference live in [`docs/`](docs/). Start with [`docs/README.md`](docs/README.md) for the index.

---

## License

MIT — see [`LICENSE`](LICENSE).
