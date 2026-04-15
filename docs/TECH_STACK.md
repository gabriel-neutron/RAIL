# TECH_STACK.md — Technology Stack Reference

## 1. Stack overview

| Layer | Technology | Version | Rationale |
|---|---|---|---|
| Desktop framework | Tauri | v2 | Rust backend + React frontend, native packaging |
| Backend language | Rust | stable | Performance, safety, librtlsdr FFI |
| Frontend language | TypeScript | 5.x | Type safety for IPC contracts |
| Frontend framework | React | 18 | Component model, hooks, canvas integration |
| State management | Zustand | 4.x | Minimal, no boilerplate |
| FFT | rustfft | 6.x | Proven, fast, pure Rust |
| RTL-SDR binding | rtlsdr-rs + raw FFI | latest | Direct hardware access |
| Capture format | SigMF | 1.0 | Community standard |
| Audio playback | Web Audio API | native | Browser API, no lib needed |
| Canvas rendering | Canvas API | native | No waterfall libs |
| Styling | CSS Modules | native | Scoped, no runtime |
| Build tool | Vite | 5.x | Fast, Tauri default |
| Linting (Rust) | clippy | bundled | Zero warnings policy |
| Linting (TS) | ESLint + strict TS | latest | No `any` policy |

---

## 2. Rust dependencies

```toml
[dependencies]
# Tauri
tauri = { version = "2", features = [] }
tauri-build = "2"

# RTL-SDR
rtlsdr = "0.1"          # rtlsdr-rs — evaluate at scaffold time
                          # fallback: raw FFI via libloading

# FFT
rustfft = "6"

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

# Optional: IIR filters for de-emphasis
# biquad = "0.4"
```

**Note on rtlsdr-rs**: evaluate crate maturity at scaffold time.
If the crate is insufficient, use raw `unsafe` FFI against the system
librtlsdr. Document the decision in a comment in `hardware/mod.rs`.

---

## 3. Frontend dependencies

```json
{
  "dependencies": {
    "@tauri-apps/api": "^2",
    "react": "^18",
    "react-dom": "^18",
    "zustand": "^4"
  },
  "devDependencies": {
    "@types/react": "^18",
    "@types/react-dom": "^18",
    "typescript": "^5",
    "vite": "^5",
    "@vitejs/plugin-react": "^4",
    "eslint": "^8"
  }
}
```

**No additional UI libraries** unless explicitly approved in `CLAUDE.md`.
No component kits (MUI, Chakra, Ant) — RAIL has a custom UI.

---

## 4. Platform prerequisites

### All platforms
- Rust toolchain (rustup, stable)
- Node.js 20+
- Tauri CLI v2 (`cargo install tauri-cli`)
- librtlsdr installed (platform-specific, see below)

### Linux
```bash
sudo apt install librtlsdr-dev librtlsdr0
# udev rules for non-root USB access:
# copy 99-rtlsdr.rules to /etc/udev/rules.d/
```

### macOS
```bash
brew install librtlsdr
```

### Windows
- Install Zadig → replace RTL-SDR driver with WinUSB
- Install librtlsdr via MSYS2 or prebuilt DLL
- Set `LIBRTLSDR_LIB_DIR` env var for build

---

## 5. Build commands

```bash
# Development (hot reload)
cargo tauri dev

# Production build
cargo tauri build

# Rust lint
cargo clippy -- -D warnings

# TypeScript check
npx tsc --noEmit

# Run tests
cargo test
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
