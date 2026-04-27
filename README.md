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
| **Live waterfall** | 25 fps, no third-party library |
| **Six demodulation modes** | WBFM, NFM, AM, USB, LSB, CW |
| **Signal meter** | dBFS level + peak hold |
| **Wideband scanner** | Sweeps a user-defined frequency range, auto-discovers active signals |
| **Signal classifier** | Three-path dispatch: frequency prior → bandwidth → modulation analysis; emits a mode suggestion |
| **SigMF capture** | IQ clips, audio recordings, waterfall screenshots |
| **Offline replay** | Full pipeline runs against saved IQ files — no hardware needed |
| **Bookmarks** | Named frequencies with one-click tune |
| **PPM correction** | Calibration offset per dongle |
> **Note:** While all features are implemented in theory, some errors remain unresolved in practice and are still being debugged.

## Technical proof points

- **Hand-written librtlsdr FFI** — direct binding via `libloading`, no `rtl_tcp` daemon
- **From-scratch Hilbert FIR phasing** — 129-tap Hilbert filter with matched group-delay compensation on both I/Q paths; used for USB and LSB demodulation
- **Three-path signal classifier** — frequency prior, bandwidth measurement (6 dB above noise floor), and envelope/asymmetry discrimination; see `docs/SIGNALS.md §5` for the full design
- **Binary IPC** — waterfall frames and audio stream sent as float32 `ArrayBuffer` Tauri events, not JSON; keeps the 25 fps budget without serialization overhead
- **Coherent-gain-correct FFT** — window normalization divides by the sum of window coefficients, not `N`; all dB readings are calibrated to 0 dBFS
- **SigMF-compliant capture** — IQ files readable by any SigMF-aware tool (GNU Radio, SigMF Python library, inspectrum)

---

## Why RAIL?

I built RAIL because I wanted to understand what makes a real SIGINT pipeline work, not just call a library and display a plot. I implemented the core components I wanted, but I know it is not professional grade yet, and some issues remain unresolved, such as ghost signals and demodulation problems.

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

**Or download the installer** — see [Releases](../../releases) for the Windows `.exe`, macOS `.dmg`, and Linux `.AppImage`.

---

## Offline demo

No dongle? The repo ships with a short IQ capture at [`docs/assets/demo_iq.sigmf-data`](docs/assets/demo_iq.sigmf-data) (101.58 MHz FM, recorded 2026-04-19). Open it from the replay transport — the waterfall, spectrum, and audio paths run end-to-end against the file with no hardware.

---

## Going deeper

Architecture, DSP math, and signal reference live in [`docs/`](docs/). Start with [`docs/README.md`](docs/README.md) for the index.

---

## Known issues

* Ghost signals often appear on untuned frequencies, typically 450 Hz away from the tuned value
* The RDS demodulator does not work
* Crashes often occur at startup

---

## License

MIT — see [`LICENSE`](LICENSE).
