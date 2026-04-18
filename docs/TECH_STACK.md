# TECH_STACK.md — Technology Stack Reference

## 1. Stack overview

| Layer | Technology | Version | Rationale |
|---|---|---|---|
| Desktop framework | Tauri | v2 | Rust backend + React frontend, native packaging |
| Backend language | Rust | stable | Performance, safety, librtlsdr FFI |
| Frontend language | TypeScript | 5.x | Type safety for IPC contracts |
| Frontend framework | React | 19 | Component model, hooks, canvas integration |
| State management | Zustand | 5.x | Minimal, no boilerplate |
| FFT | rustfft | 6.x | Proven, fast, pure Rust |
| RTL-SDR binding | hand-written FFI (`src/hardware/ffi.rs`) | librtlsdr | Direct hardware access, no `bindgen` |
| Capture format | SigMF | 1.0 | Community standard |
| Audio playback | Web Audio API | native | Browser API, no lib needed |
| Canvas rendering | Canvas API | native | No waterfall libs |
| Styling | plain CSS | native | Scoped via class names, no runtime |
| Build tool | Vite | 7.x | Fast, Tauri default |
| Linting (Rust) | clippy | bundled | Zero warnings policy |
| Linting (TS) | ESLint + strict TS | latest | No `any` policy |

---

## 2. Rust dependencies

```toml
[dependencies]
# Tauri
tauri = { version = "2", features = [] }
tauri-build = "2"

# RTL-SDR: hand-written FFI, no crate — see src-tauri/src/hardware/ffi.rs

# FFT
rustfft = "6"
num-complex = "0.4"

# Byte-level helpers
bytemuck = "1"

# USB enumeration (device detection)
nusb = "0.1"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Async runtime (Tauri uses tokio internally)
tokio = { version = "1", features = ["full"] }

# Logging
log = "0.4"
env_logger = "0.11"

# Error handling
thiserror = "1"

# Tauri plugins
tauri-plugin-dialog = "2"
```

**Note on RTL-SDR binding**: RAIL uses a hand-written `unsafe` FFI against
the system `librtlsdr` (see `src-tauri/src/hardware/ffi.rs`). No `bindgen`,
no third-party wrapper crate — the surface is small and stable.

---

## 3. Frontend dependencies

```json
{
  "dependencies": {
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-dialog": "^2",
    "react": "^19",
    "react-dom": "^19",
    "zustand": "^5"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2",
    "@types/react": "^19",
    "@types/react-dom": "^19",
    "typescript": "~5.8",
    "vite": "^7",
    "@vitejs/plugin-react": "^4"
  }
}
```

**No additional UI libraries** unless explicitly approved in `CLAUDE.md`.
No component kits (MUI, Chakra, Ant) — RAIL has a custom UI.

---

## 4. Platform prerequisites

### All platforms
- Rust toolchain (rustup, stable, 1.85+)
- Node.js 20+
- Tauri CLI v2 (bundled via `@tauri-apps/cli` devDep — no `cargo install` needed)
- librtlsdr installed (platform-specific, see below)

### Linux (Debian / Ubuntu)
```bash
sudo apt install librtlsdr-dev librtlsdr0 \
                 libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
                 libssl-dev libgtk-3-dev libsoup-3.0-dev \
                 build-essential curl wget pkg-config
# Non-root USB access: see the udev rules block in the root README.
```

### macOS
```bash
brew install librtlsdr
```

### Windows
- Install [Zadig](https://zadig.akeo.ie/) → replace the dongle's driver
  with **WinUSB** (see `HARDWARE.md` §6).
- Drop the librtlsdr prebuilts (`rtlsdr.dll`, `rtlsdr.lib`,
  `pthreadVC2.dll`, `msvcr100.dll`) into `vendor/librtlsdr-win-x64/`,
  or set `LIBRTLSDR_LIB_DIR` to a folder containing them. The build
  script copies the runtime DLLs next to the target binary automatically.

---

## 5. Build commands

```bash
# Install JS deps (first run only)
npm install

# Development (hot reload)
npm run tauri dev

# Production build (bundles installers under src-tauri/target/release/bundle/)
npm run tauri build

# Rust lint
cargo clippy --all-targets -- -D warnings

# TypeScript check
npx tsc --noEmit

# Frontend build
npm run build

# Rust tests
cargo test --lib
```

---

## 6. Key architecture decisions and rationale

### Why Tauri over Electron?
- Rust backend = direct hardware access, no Node.js subprocess
- Binary size: Tauri ~10MB vs Electron ~100MB+
- Performance: Rust DSP on main data path
- Security: smaller attack surface

### Why direct librtlsdr FFI over rtl_tcp?
- Eliminates external process dependency
- Lower latency (no TCP round-trip)
- More control over buffer sizes and error handling
- Better demonstration of hardware-level knowledge

### Why Web Audio API over Rust system audio?
- Cross-platform without crate evaluation risk
- Audio timing handled by browser engine
- Simpler to implement and debug
- Tauri WebView has full Web Audio support

### Why binary Tauri events over JSON for streaming?
- JSON serialization of 2048 floats at 25fps = ~12MB/s overhead
- Binary ArrayBuffer = ~200KB/s at same rate
- No parsing overhead on React side
- `Float32Array` wraps ArrayBuffer directly, zero copy

### Why rustfft over FFTW?
- Pure Rust, no C dependency to manage
- Sufficient performance for RTL-SDR data rates
- Simpler build chain across platforms
- FFTW would add significant build complexity for marginal gain
