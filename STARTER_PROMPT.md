# RAIL — Claude Code Starter Prompt (Phase 0)

Paste this prompt as your first message to Claude Code in the project directory.

---

## PROMPT

You are starting work on **RAIL (Radio Analysis and Intel Lab)**, a Tauri v2
desktop application for RTL-SDR signal reception and analysis.

**Read these files before doing anything else — in this order:**
1. `CLAUDE.md` — core rules, non-negotiables, approved libraries
2. `docs/README.md` — documentation index and ownership map
3. `docs/TECH_STACK.md` — full stack, dependencies, build commands
4. `docs/ARCHITECTURE.md` — module structure, IPC contract, threading model
5. `docs/HARDWARE.md` — RTL-SDR specifics and librtlsdr notes
6. `docs/TIMELINE.md` — development phases (we are on Phase 0)

Do not write any code until you have read all six files.

---

## YOUR TASK: Phase 0 — Project scaffold and prerequisites

Work through this checklist **in order**. Do not skip steps.
After each step, confirm it works before proceeding to the next.

### Step 1 — Verify host prerequisites

Check and report the status of each:
- [ ] Rust toolchain: `rustc --version`, `cargo --version`
- [ ] Tauri CLI v2: `cargo tauri --version` (install if missing: `cargo install tauri-cli`)
- [ ] Node.js: `node --version` (must be 20+)
- [ ] npm/pnpm: `npm --version`
- [ ] librtlsdr: check if installed (Linux: `dpkg -l | grep rtlsdr`, macOS: `brew list | grep rtlsdr`, Windows: check manually)
- [ ] RTL-SDR device: plugged in via USB — detect with `rtl_test -t` if available

Report any missing prerequisites with the exact install command for the detected platform.
**Do not proceed to Step 2 if prerequisites are missing.**

### Step 2 — Initialize Tauri v2 project

```bash
cargo create-tauri-app rail --template react-ts --manager npm
cd rail
```

If `create-tauri-app` is not installed:
```bash
cargo install create-tauri-app
```

After initialization, verify:
- `cargo tauri dev` launches without error (you can close the window)
- `cargo build` inside `src-tauri/` passes with no errors
- React frontend loads in the Tauri window

### Step 3 — Add Rust dependencies

Edit `src-tauri/Cargo.toml` to add:

```toml
[dependencies]
rustfft = "6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
log = "0.4"
env_logger = "0.11"
thiserror = "1"
```

For RTL-SDR: attempt to add `rtlsdr` crate first:
```toml
rtlsdr = "0.1"
```

Run `cargo build`. If `rtlsdr` crate fails to compile (librtlsdr not found or
crate issues), document the exact error and propose the fallback strategy
(raw FFI via `libloading`). Do not guess — report the exact build output.

### Step 4 — Add frontend dependencies

```bash
npm install zustand
npm install @tauri-apps/api
```

Verify no peer dependency warnings that indicate version conflicts.

### Step 5 — Create project folder structure

Create the Rust module structure under `src-tauri/src/`:

```
src-tauri/src/
  main.rs              (already exists — do not delete)
  lib.rs               (Tauri entry point)
  error.rs             (RailError enum — see ARCHITECTURE.md §6)
  hardware/
    mod.rs
    stream.rs
  dsp/
    mod.rs
    fft.rs
    waterfall.rs
    demod/
      mod.rs
      fm.rs
      am.rs
    filter.rs
  capture/
    mod.rs
    sigmf.rs
    session.rs
  ipc/
    mod.rs
    commands.rs
    events.rs
```

Create the React structure under `src/`:

```
src/
  components/
    Waterfall/
      index.tsx
    FrequencyControl/
      index.tsx
    ModeSelector/
      index.tsx
    FilterControl/
      index.tsx
    SignalMeter/
      index.tsx
    AudioControls/
      index.tsx
    CapturePanel/
      index.tsx
  store/
    radio.ts
    session.ts
  hooks/
    useWaterfall.ts
    useAudio.ts
  ipc/
    commands.ts
    events.ts
  App.tsx               (already exists — restructure if needed)
```

Each new Rust file: create with a module doc comment and empty stub.
Each new React file: create with a minimal functional component or empty export.

### Step 6 — Implement RailError

In `src-tauri/src/error.rs`, implement the `RailError` enum as specified
in `docs/ARCHITECTURE.md §6`. Derive `thiserror::Error` for clean error messages.

### Step 7 — End-to-end ping test

Implement a single Tauri command to verify IPC works:

**Rust** (`ipc/commands.rs`):
```rust
#[tauri::command]
pub fn ping() -> &'static str {
    "pong"
}
```

**React** (`ipc/commands.ts`):
```typescript
import { invoke } from '@tauri-apps/api/core';
export const ping = (): Promise<string> => invoke('ping');
```

Call `ping()` in `App.tsx` on mount and log the result to console.
Verify "pong" appears in the Tauri dev console.

### Step 8 — RTL-SDR device detection

In `hardware/mod.rs`, implement a function that:
1. Calls `get_device_count()` from librtlsdr
2. If count > 0: logs device name and index
3. If count == 0: returns `Err(RailError::DeviceNotFound)`

Expose this as a Tauri command: `check_device() -> Result<DeviceInfo, RailError>`
where `DeviceInfo` is a serializable struct with `{ index: u32, name: String }`.

Call it from React on startup and log the result.

**If librtlsdr is not available on this system**: do not mock it silently.
Return `Err(RailError::DeviceNotFound)` with a clear error message, and
display a "No RTL-SDR device found" message in the UI.

### Step 9 — Verify Phase 0 exit criterion

Phase 0 is complete when ALL of the following are true:
- [ ] `cargo tauri dev` launches without errors
- [ ] Rust compiles with zero `clippy` warnings (`cargo clippy -- -D warnings`)
- [ ] TypeScript compiles with zero errors (`npx tsc --noEmit`)
- [ ] Ping command returns "pong" in the console
- [ ] RTL-SDR device is detected and logged (or DeviceNotFound error is shown cleanly)
- [ ] All module stubs are created and compile
- [ ] `/docs/` folder and all documentation files are in place

---

## RULES TO FOLLOW THROUGHOUT

1. Read `CLAUDE.md` rules apply at all times
2. No `unwrap()` in non-test code
3. No `any` in TypeScript
4. If you encounter a DSP question, stop and read `docs/DSP.md`
5. If you need to modify `/docs/`, read the documentation rules in `CLAUDE.md` first
6. Do not implement Phase 1 features during Phase 0
7. Report blockers explicitly — do not silently work around hardware issues

---

## IF YOU ARE BLOCKED

If any step fails and you cannot resolve it:
1. Report the exact error message
2. State which file/doc you consulted
3. Propose two options (not one) and ask which to proceed with

Do not guess on DSP math or hardware behavior — flag it and reference the docs.
