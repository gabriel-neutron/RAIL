# Performance profiling (runtime)

Notes for optional waterfall optimization, Rust emit profiling, and UI/CSS measurement.

## 1. Waterfall UI: `drawWaterfallRow` (colormap Worker gate)

**Decision rule:** add a Web Worker for the colormap LUT loop **only if** main-thread profiling shows the LUT segment at **≥ ~1 ms per frame** at **FFT = 8192**.

**Current status:** **NO-GO** until a live or long replay session records p95 LUT time ≥ ~1 ms/frame. A Web Worker was **not** added; re-open only when measurements justify `postMessage` overhead.

**CSS:** remove unused rules using Chromium Coverage (§4 below), not only by grep.

### 1.1 Dev: localStorage profiling helper

In **dev** (`import.meta.env.DEV`), set in DevTools console:

```js
localStorage.setItem("rail_profile_waterfall", "1");
```

Reload. While streaming or replaying IQ, the app logs **rolling averages every ~60 frames** to the console:

- `lut ms` — colormap loop only (`Waterfall/index.tsx`)
- `blit ms` — `drawImage` scroll + `putImageData`

Clear with:

```js
localStorage.removeItem("rail_profile_waterfall");
```

### 1.2 Chrome / Edge Performance panel

1. Open DevTools → **Performance**, record ≥60 s with live stream or IQ replay at FFT 8192.
2. Inspect long tasks on the main thread; locate `drawWaterfallRow` / waterfall frame handler.
3. Record **p95** time for the LUT portion vs full row draw.

**When you have numbers:** add p95 LUT ms, p95 full `drawWaterfallRow` ms, scenario (live vs replay), zoom level (optional: paste into a PR or issue).

## 2. Waterfall fps

Optional: DevTools **Rendering** → FPS, or a custom counter. Aim to confirm **≥25 fps p95** over 60 s or list contributors.

## 3. Rust emit path (`profile` feature)

Build with `cargo build --features profile` (or enable the feature in your Tauri build). Set **`RAIL_PROFILE=1`** and e.g. **`RUST_LOG=rail_perf=info`** (or `RUST_LOG=info`). Summaries log every few seconds from `perf_emit.rs` when the logger level allows.

## 4. CSS coverage

Chromium **Coverage** (Ctrl+Shift+P → “coverage”) on a full UI walkthrough; compare against `src/App.css`.
