# RAIL — V1 Review Report (pre-Phase 7)

> Companion to the Review Plan at `.cursor/plans/rail_review_plan_3f5195b6.plan.md`.
> Static review, grounded in verification commands run on Windows (PowerShell).
> Raw command outputs archived under `review-artifacts/`.

## Table of contents

1. [Executive summary](#1-executive-summary)
2. [Verification log](#2-verification-log)
3. [Scorecard (B.1–B.17)](#3-scorecard-b1-b17)
4. [Hot-path analysis](#4-hot-path-analysis)
5. [Hygiene findings](#5-hygiene-findings)
6. [Correctness findings](#6-correctness-findings)
7. [Security / privacy](#7-security--privacy)
8. [Tests & CI](#8-tests--ci)
9. [Docs vs code drift](#9-docs-vs-code-drift)
10. [Suggested inline comment placeholders](#10-suggested-inline-comment-placeholders)
11. [Remediation backlog](#11-remediation-backlog)
12. [Open questions carried forward](#12-open-questions-carried-forward)

---

## 1. Executive summary

RAIL is in much better shape than a typical "pre-V2" codebase. Verification is green across the board (clippy clean, 36/36 Rust tests pass, TypeScript typechecks, `npm audit` clean, frontend bundle 75 KB gzipped). The hot paths are deliberate: FFT planner, Hann window and scratch are reused; the colormap is a precomputed 256-entry LUT; the DSP chain keeps its scratch buffers across calls; the frontend drains binary channels off rAF with a budget. FFI is exceptionally well-documented — every `unsafe` block carries a SAFETY comment, `Send`/`Sync` impls are justified in code, and the WinUSB disconnect quirk is handled intentionally instead of papered over.

Top three wins (ordered by cost-to-land):

- **S1. Kill per-frame Vec allocations in the DSP worker.** `emit_waterfall_frames` and `emit_audio_chunks` each allocate *two* `Vec`s per emission (`drain(..N).collect()` and `bytes.to_vec()`), i.e. ~100 small allocations per second of streaming. Replace the `drain().collect()` with a scratch buffer owned by `DspTaskCtx` (the `bytes.to_vec()` is forced by Tauri's `InvokeResponseBody::Raw(Vec<u8>)` API and can't go, but the upstream allocation can). See §4.1.
- **S2. Resolve the `docs/ARCHITECTURE.md` §3 IPC contract drift.** The doc describes `waterfall-frame` and `audio-chunk` as named Tauri events, but the implementation uses per-session `tauri::ipc::Channel<InvokeResponseBody>` passed to `start_stream`/`start_replay`. Same for the commands table (doc says `set_frequency`, code says `retune`; doc says `save_iq_clip`, code says `start_iq_capture`/`stop_iq_capture`/`finalize_iq_capture`). See §9.
- **S3. Split `src-tauri/src/ipc/commands.rs` (1611 lines) by domain** before Phase 7 adds more commands. Three sub-files cleanly isolate: lifecycle/tune, capture, replay, plus an internal `dsp_task.rs` for the worker. Move-only, no behaviour change. See §5.3.

Top three risks to dismiss or convert to facts with measurement in a follow-up session (see §4.3):

- **R1.** End-to-end waterfall latency and jitter at 25 fps — no runtime measurement in this session; Rust side limits emit to 40 ms, but the rAF drain and canvas `drawImage` self-copy haven't been profiled.
- **R2.** Audio underruns across a 10-minute session — `useAudio` uses a one-`AudioBuffer`-per-chunk approach that is correct but GC-sensitive.
- **R3.** CSS bloat: `src/App.css` is 974 lines / 126 selectors for 12 components. Likely contains dead rules; needs a Chromium Coverage pass.

Dismissed hypotheses from the plan's risk register (evidence in §3):

- **H3 dismissed** — colormap LUT is precomputed at 256 entries and memoized in `src/components/Waterfall/index.tsx:44`.
- **H5 dismissed** — every `unwrap()` / `expect()` / `panic!` in `src-tauri/src/` is inside a `#[cfg(test)]` module. The Phase 6 exit criterion *"No `unwrap()` in non-test code"* holds.
- **M4 dismissed** — Zustand selectors in `src/store/radio.ts` are used with scalar accessors (`(s) => s.volume`), not object returns. No render churn from the store shape.

## 2. Verification log

### 2.1 Commands run and raw results

All commands run from `c:\Users\antoi\Documents\Netechoppe\RAIL` on Windows 10 / PowerShell. Raw outputs in `review-artifacts/`.

| Command | Result | Artifact |
| --- | --- | --- |
| `cd src-tauri; cargo clippy --all-targets -- -D warnings` | **pass** (zero warnings, finished in 21 s) | `review-artifacts/clippy.txt` |
| `cd src-tauri; cargo test --all-targets --no-fail-fast` | **36 passed, 0 failed, 0 ignored** | `review-artifacts/cargo-test.txt` |
| `cd src-tauri; cargo tree -d` | duplicates present but **all transitive via Tauri** (`bitflags` 1/2, `log` 0.4.29 twice from `env_logger` vs `nusb`, `getrandom` 0.1/0.2/0.3/0.4, `phf` 0.8/0.10/0.11, `windows-sys` 0.48/0.59/0.60/0.61, etc.). The `rail` crate's direct deps are clean. | `review-artifacts/cargo-tree-d.txt` |
| `npx tsc --noEmit` | **pass** (no output) | `review-artifacts/tsc.txt` |
| `npm run build` | **pass**: `dist/assets/index-*.js` 238.59 kB / 75.39 kB gzipped; CSS 12.92 kB / 2.82 kB gzipped; build 733 ms | `review-artifacts/npm-build.txt` |
| `npm audit --omit=dev` | **found 0 vulnerabilities** | `review-artifacts/npm-audit.txt` |
| `cargo audit` | **not run** (tool not installed locally). See §8.3 for suggested CI addition. | — |
| `rg "unwrap\(\)\|\.expect\(\|panic!\|todo!\|unimplemented!" src-tauri/src` | 42 hits; **all inside `#[cfg(test)]` modules** (see §5.1) | inline |
| `rg "TODO\|FIXME\|XXX\|HACK" src src-tauri/src` | **0 hits** in both trees | inline |

### 2.2 Environment

- OS: Windows 10 / 11 (win32 10.0.26200), PowerShell.
- Rust toolchain: stable, already configured for Tauri v2.
- Node: present, able to run `npm ci` / `vite build` in ~5 s.
- No live RTL-SDR session was exercised in this review — runtime performance numbers in §4 are **static analysis only** and flagged as such.

## 3. Scorecard (B.1–B.17)

Scale: 1 (blocker) → 5 (teachable best-practice). Citations follow `path:line` form.

| # | Dimension | Score | Evidence |
| :---: | --- | :---: | --- |
| B.1 | Hot-path allocation (Rust) | **3.5** | FFT processor is alloc-free per call (`src-tauri/src/dsp/fft.rs:36-44` preallocates window/buffer/scratch/out). `FrameBuilder` reuses its IQ scratch (`src-tauri/src/dsp/waterfall.rs:78-84`). `DemodChain` scratches are reused (`src-tauri/src/dsp/demod/mod.rs:122-124`). **But** the DSP worker allocates two `Vec`s per emission: `src-tauri/src/ipc/commands.rs:1505` (`drain(..FFT_SIZE).collect()`), `src-tauri/src/ipc/commands.rs:1537` (same for audio), `src-tauri/src/ipc/commands.rs:1514` and `1539` (`bytes.to_vec()` forced by `InvokeResponseBody::Raw`). |
| B.2 | Backpressure & channel sizing | **4** | IQ: bounded `mpsc::channel::<DspInput>(IQ_CHANNEL_CAPACITY=8)` (`src-tauri/src/hardware/stream.rs:23`) with **try_send + drop counter** in the FFI callback (`src-tauri/src/hardware/stream.rs:70-75`, logs at power-of-two thresholds). Control & capture channels are unbounded (`src-tauri/src/ipc/commands.rs:262-263`) which is fine given human-rate message volumes. Demod control + capture: unbounded — acceptable because they are user-initiated and carry no bulk data. |
| B.3 | Event payload sizing & cadence | **4** | Waterfall channel: `FFT_SIZE=2048 × 4 B = 8 KB` payload at ≤25 fps (`src-tauri/src/ipc/commands.rs:38,45`). Audio channel: `AUDIO_CHUNK_SAMPLES=1764 × 4 B ≈ 7 KB` at ~25 chunks/s (`src-tauri/src/ipc/commands.rs:58`). `signal-level` JSON at 25 Hz cap (`src-tauri/src/ipc/commands.rs:49`) with peak-decay that reads naturally. Numbers are all in-bounds for the 200 kB/s class of traffic. |
| B.4 | FFT / window reuse | **5** | `FftPlanner::plan_fft_forward` called once (`src-tauri/src/dsp/fft.rs:33-34`), scratch from `get_inplace_scratch_len()` preallocated (`src-tauri/src/dsp/fft.rs:35,40`), Hann window precomputed (`src-tauri/src/dsp/fft.rs:38` via `hann_window(n)`). FFT shift is in-place `rotate_left` (`src-tauri/src/dsp/fft.rs:85`). |
| B.5 | FFI soundness | **5** | Every `unsafe` block has a SAFETY comment. `Send`/`Sync` decisions documented and scoped (`src-tauri/src/hardware/mod.rs:112-115,326-330`, `src-tauri/src/hardware/stream.rs:95-98`). FFI panic-safety via `panic::catch_unwind(AssertUnwindSafe(...))` on the C callback (`src-tauri/src/hardware/stream.rs:53`). `Canceler` is intentionally distinct from `RtlSdrDevice` so ownership is single-threaded while cancellation is cross-thread. WinUSB disconnect-on-close segfault is **handled with an intentional `std::mem::forget` + rationale comment** (`src-tauri/src/hardware/stream.rs:194-198`). |
| B.6 | Async / threading hygiene | **4.5** | Stream reader is a dedicated `std::thread` (`src-tauri/src/hardware/stream.rs:161-202`) — correct given `rtlsdr_read_async` blocks. DSP task uses `tokio::task::spawn_blocking` + `blocking_recv` (`src-tauri/src/ipc/commands.rs:1214,1276`) — correct because DSP is CPU-bound. Stop path is explicit and idempotent: `IqStream::Drop` cancels+joins (`src-tauri/src/hardware/stream.rs:235-245`), `stop_stream` takes the session out first, then awaits both the stream+DSP handles (`src-tauri/src/ipc/commands.rs:334-360`). Replay path mirrors this. |
| B.7 | Error surfaces | **5** | `RailError` has 6 variants (`src-tauri/src/error.rs:12-30`), serialized as `{kind, message}` via `#[serde(tag = "kind", content = "message")]`. Frontend mirrors exactly (`src/ipc/commands.ts:8-19`). `session_poisoned` helper centralizes mutex-poison handling (`src-tauri/src/ipc/commands.rs:630`). Zero `unwrap()` / `expect()` / `panic!()` in non-test code (verified — see §5.1). |
| B.8 | React render churn | **4** | Store selectors are all scalar (`src/store/radio.ts:36-71` typed, `s.volume`, `s.muted`, `s.zoom` accessors). Frame callback ref-held (`src/hooks/useWaterfall.ts:66-75`) so `onFrame` changes don't re-tear the channel. Waterfall drags use a `ref` guard (`src/components/Waterfall/index.tsx:81`) to avoid mid-drag redraws. Canvas reset effect depends only on `session?.fftSize`, `zoom`, `waterfallEpoch` — explicit and minimal (`src/components/Waterfall/index.tsx:95-105`). |
| B.9 | Buffer lifecycle in the UI | **4** | Waterfall channel buffers wrapped via `new Float32Array(buffer)` once and pushed to a `pending` queue (`src/hooks/useWaterfall.ts:94-99`). rAF drain has a **360-frame budget** (`src/hooks/useWaterfall.ts:113`) so a prefill burst can't stall the main thread. `ImageData` row buffer is created once per-width and reused (`src/components/Waterfall/index.tsx:303-308`). No obvious retention beyond the pending queue; memory-over-time still unmeasured — see §4.3. |
| B.10 | State ergonomics | **4** | Three Zustand stores (`radio`/`capture`/`replay`) are small and single-responsibility. Debounced tune (30 ms) and debounced commands (60 ms) live in the store (`src/store/radio.ts:73-124`) — a good spot since the UI side already knows which source is live. Replay vs live concerns are separated cleanly (`src/store/replay.ts`, and `setFrequency` early-returns if replay is active — `src/store/radio.ts:146`). |
| B.11 | Naming & module boundaries | **3** | Rust module tree is clean (`dsp/` → `demod/`, `hardware/`, `capture/`, `ipc/`) and matches `docs/ARCHITECTURE.md §2`. **Exception:** `src-tauri/src/ipc/commands.rs` is 1611 lines and mixes command handlers with the DSP worker (`spawn_dsp_task`, `DspTaskCtx`, `emit_*`) plus capture helpers (`iso8601_compact`, `civil_from_days`, `suggested_name`). See §5.3 for a move-only split proposal. |
| B.12 | Dead code / TODO hygiene | **5** | **Zero** `TODO` / `FIXME` / `XXX` / `HACK` hits across `src/` and `src-tauri/src/`. Phase 6 Pre-release hygiene is visible. |
| B.13 | Documentation accuracy | **2.5** | `docs/ARCHITECTURE.md §3` is materially wrong about the streaming IPC: it names `waterfall-frame` / `audio-chunk` events, but the implementation uses per-session `tauri::ipc::Channel`s passed to `start_stream` / `start_replay`. The commands table in the doc is also out of date (see §9). `docs/TIMELINE.md` Phase 6 claims are all truthful after re-verification. |
| B.14 | Security / privacy footguns | **4** | `src-tauri/capabilities/default.json` permissions are minimal (`core:default`, `dialog:allow-save`, `dialog:allow-open`). `env_logger::try_init()` with default level — silent unless `RUST_LOG` is set (no PII leak at info). FFI is sound. SigMF / WAV finalization uses a **temp path + move** pattern via `capture/tmp.rs`+`finalize_capture` (`src-tauri/src/ipc/commands.rs:886-923`). PNG save uses an atomic `write-then-rename` (`src-tauri/src/ipc/commands.rs:961-965`). Bookmarks use an atomic temp+rename+fsync write (`src-tauri/src/bookmarks.rs:75-93`). **One finding**: `tauri.conf.json` has `security.csp: null` (`src-tauri/tauri.conf.json:21`) — fine for a local app that loads only the bundled frontend, but worth switching to an explicit restrictive string for the portfolio. |
| B.15 | CSS/UI weight | **3** | `src/App.css` is 974 lines, 126 selectors for 12 components (measured). Production bundle gzips to 2.82 KB so the user-facing impact is low, but the source is a sanitize target before Phase 7 UI work. Not inspected in detail this pass — see §4.3 coverage experiment. |
| B.16 | Tests vs risk | **4** | DSP: solid — FFT (`dc_input_peaks_at_center_after_shift`, `shift_is_rotation`), window (`hann_*`), resampler (`linear_resampler_*`), demod chain end-to-end (`chain_fm_demodulates_tone`, `chain_am_demodulates_amplitude`, `chain_squelch_silences_audio`, `chain_rms_dbfs_is_monotonic_in_amplitude`), fs/4 shift math (`fs4_shift_cycles_correctly`, `fs4_shift_moves_dc_off_center`). Capture: WAV header + round-trip, SigMF round-trip with meta. Bookmarks: load-missing + save/load round-trip. Replay: path swap, clamping. **Gaps**: no tests covering `set_bandwidth` reconfigure continuity (`DemodChain::apply` on bandwidth change flushes the FIR delay line — should be tested), no tests for `emit_waterfall_frames` behaviour on backlog vs rate-limit, no IPC smoke test with a mock `Channel`. |
| B.17 | CI coverage vs risk | **3** | `.github/workflows/ci.yml` runs clippy + `cargo test --lib` (note: **not** `--all-targets`) + tsc + vite build on Ubuntu. **Gaps**: no `cargo fmt --check`, no `cargo audit`, no `cargo deny`, no `npm audit`, no Windows or macOS job (Windows is the *only* verified target per `README.md`), no lint script defined at all in `package.json`. See §8.3. |

## 4. Hot-path analysis

### 4.1 Waterfall pipeline (static evidence)

End-to-end:

```
RTL-SDR (USB callback, librtlsdr thread)
  └── src-tauri/src/hardware/stream.rs:52  on_iq
        ├── slice.to_vec()                          <-- alloc #1 (per USB buffer, unavoidable handoff)
        └── mpsc::try_send(DspInput::RtlU8(owned))
tokio::task::spawn_blocking worker (src-tauri/src/ipc/commands.rs:1214)
  └── DspTaskCtx::run  (src-tauri/src/ipc/commands.rs:1267)
        ├── self.shifted.resize(n_complex, ...)     (amortized, keeps capacity)
        ├── iq_u8_to_complex(&chunk, &mut self.shifted)
        ├── apply_fs4_shift(&mut self.shifted, ...)
        └── emit_waterfall_frames  (1496)
              ├── self.fft_pending.extend_from_slice(&self.shifted)      (amortized)
              ├── while fft_pending.len() >= FFT_SIZE:
              │     let frame: Vec<Complex<f32>> =
              │         self.fft_pending.drain(..FFT_SIZE).collect();    <-- alloc #2 (PER FRAME)
              │     self.builder.process_shifted(&frame)                 (alloc-free)
              │     channel.send(InvokeResponseBody::Raw(bytes.to_vec()))<-- alloc #3 (PER FRAME, forced by API)
Frontend (main thread, src/hooks/useWaterfall.ts:94)
  └── waterfallChannel.onmessage
        └── pending.push(new Float32Array(buffer))
rAF drain (src/hooks/useWaterfall.ts:107)
  └── for up to 360 per tick: handler(pending.shift()!)
Waterfall component (src/components/Waterfall/index.tsx:86)
  └── onFrame = rawFrame => {
        const frame = cropCenter(rawFrame, zoomRef.current)   (subarray, no copy)
        drawWaterfallRow(canvas, frame, rowImageRef, lut)
        drawSpectrum(spectrumCanvas, frame)
      }
drawWaterfallRow (src/components/Waterfall/index.tsx:286)
  ├── ImageData reused via rowImageRef
  ├── LUT-indexed fill (loop of frame.length)
  ├── ctx.drawImage(canvas, 0,0,W,H-1, 0,1,W,H-1)   <-- canvas self-scroll, 1px per frame
  └── ctx.putImageData(row, 0, 0)
```

Findings:

- **Per-frame Rust-side allocations: 2.** `drain(..FFT_SIZE).collect()` at `src-tauri/src/ipc/commands.rs:1505` and `bytes.to_vec()` at `:1514`. The same pair exists in `emit_audio_chunks` (`:1537`, `:1539`). Upper bound: `(waterfall 25 fps + audio ≈25 chunks/s) × 2 allocs = ~100 small allocs/s`. Each is a modest-sized `Vec` (8 KB / 7 KB), so the *bytes* aren't scary, but modern allocators still serialize; on a debug build this shows up in `perf` as `__rust_alloc` time.
- **Fix shape (for the follow-up session, not this pass):** add a `frame_buf: Vec<Complex<f32>>` to `DspTaskCtx` with capacity `FFT_SIZE`, reuse across iterations. Replace `let frame = ... .collect()` with `self.frame_buf.clear(); self.frame_buf.extend(self.fft_pending.drain(..FFT_SIZE));` and pass `&self.frame_buf`. Audio path symmetric.
- **What we can't remove without Tauri changes:** `InvokeResponseBody::Raw(Vec<u8>)` takes an owned Vec by value, so the `bytes.to_vec()` copy is forced by the current Tauri 2.10 API. Leave a comment and move on.
- **Other paths looked at and cleared:** `FftProcessor::process` (`fft.rs:56`) iterates precomputed buffers (window, scratch, out) — no allocs. `DemodChain::process` (`demod/mod.rs:178`) uses `.clear()` + `reserve` on `self.raw_audio`/`self.filtered_audio` — amortized. `FrameBuilder::process_shifted` (`waterfall.rs:115`) returns a slice view into `FftProcessor::out_db` — no alloc.

### 4.2 Audio pipeline (static evidence)

- `useAudio.enqueue` creates one `AudioBuffer` per chunk (`src/hooks/useAudio.ts:116`) and writes via `getChannelData(0).set(frame)`. At 25 chunks/s this is 25 `AudioBuffer`s/s. It's idiomatic Web Audio, but GC-sensitive. Non-blocker.
- Scheduling uses an 80 ms lookahead + 400 ms max-drift reset (`src/hooks/useAudio.ts:20-25,128-133`). The drift reset is important: without it a backgrounded tab accumulates a minute of queued audio that then blasts out on focus. Good.
- Muted/volume go through one `GainNode` set from `useRadioStore.muted`/`volume`; the effect at `:70-74` updates it in place — no node churn.

### 4.3 Measurements (not performed — deferred to a runtime session)

This review is static. The experiments below are the **minimum** set a follow-up session should run with a live RTL-SDR attached, in the order given. Each has a stop condition so the review report doesn't grow unbounded.

1. **Waterfall fps p50/p95 over 60 s live.** Stop once ≥25 fps p95 is confirmed, or 3 contributors identified.
2. **`performance.memory` over 10 min FM playback.** Stop once slope is flat (< 2 MB / min) or a retainer is found via DevTools Memory snapshot diff.
3. **Audio underruns over 10 min.** Count gaps > 20 ms in `nextStartRef - ctx.currentTime`. Stop at 0 or at first cluster > 3.
4. **CSS coverage on a full interaction session.** Use Chromium Coverage, capture % unused with top 3 dead selectors by byte size.

Expected artifacts (to attach to this doc in a later pass): one Performance profile PNG, one Memory timeline PNG, one Coverage PNG, and a short `review-artifacts/perf.md` with the four numbers.

## 5. Hygiene findings

### 5.1 Panic/unwrap inventory (non-test only)

**Result: zero non-test `unwrap()` / `expect()` / `panic!` / `todo!` / `unimplemented!`**. Phase 6 exit criterion holds.

Evidence: `rg "unwrap\(\)|\.expect\(|panic!|todo!|unimplemented!" src-tauri/src` found 42 hits (summary counts per file: `ipc/commands.rs` 1, `capture/wav.rs` 19, `capture/sigmf.rs` 12, `dsp/filter.rs` 2, `dsp/fft.rs` 2, `dsp/waterfall.rs` 4, `dsp/demod/am.rs` 1, `bookmarks.rs` 5, `replay.rs` 4). Cross-referencing against `#[cfg(test)]` module boundaries (`src-tauri/src/ipc/commands.rs:1590`, `capture/wav.rs:183`, `capture/sigmf.rs:179`, `dsp/filter.rs:279`, `dsp/fft.rs:88`, `dsp/waterfall.rs:127`, `dsp/demod/am.rs:46`, `bookmarks.rs:169`, `replay.rs:429`) confirms every hit is ≥ the corresponding `#[cfg(test)]` line, i.e. all in test modules.

There is also a soft `assert!(n > 1, ...)` in `src-tauri/src/dsp/fft.rs:32` and `assert!(input_rate_hz > BASEBAND_RATE_HZ)` in `src-tauri/src/dsp/demod/mod.rs:102`. Both are constructor-time pre-conditions on API inputs that are only ever called with compile-time constants (`FFT_SIZE=2048`, sample rate 2.048 MHz) — not runtime-dependent. Keep as-is, but consider downgrading to `debug_assert!` if you want release builds to be fully panic-free.

### 5.2 Dead code + TODO triage

- `TODO/FIXME/XXX/HACK`: **zero hits** in frontend and backend trees.
- `#[allow(clippy::too_many_arguments)]` on `spawn_dsp_task` (`src-tauri/src/ipc/commands.rs:1203`) — acceptable now, resolved naturally by the split proposal in §5.3 (the function moves into a struct with its own `new(...)`).
- No `#[allow(dead_code)]` in the crate — good.

### 5.3 Module boundaries: `ipc/commands.rs` split proposal (move-only)

`src-tauri/src/ipc/commands.rs` currently holds 7 concerns in 1611 lines. Suggested three-file split along clean seams (no behaviour change, no new types, no cross-concern leakage):

```
src-tauri/src/ipc/
├── mod.rs                    (unchanged)
├── events.rs                 (unchanged, 83 lines)
├── commands.rs               (~520 lines after split)
│     keeps: AppState, register(), ping, check_device,
│     start_stream, stop_stream, set_gain, available_gains, retune,
│     set_ppm, set_mode, set_bandwidth, set_squelch, send_control,
│     bookmarks commands + args, session_poisoned, Session/SessionSource
│     (internal session types stay here because every other file borrows them)
├── capture_cmd.rs            (~400 lines, NEW)
│     moves: CaptureControl enum, AudioStopInfo, IqStopInfo,
│     iso8601_compact, civil_from_days, now_secs, suggested_name,
│     radio_snapshot, capture_sender,
│     start_audio_capture, stop_audio_capture,
│     start_iq_capture, stop_iq_capture,
│     finalize_capture, finalize_iq_capture, discard_capture,
│     screenshot_suggestion, save_screenshot,
│     + the capture-related tests (iso8601_*, suggested_name_*)
├── replay_cmd.rs             (~250 lines, NEW)
│     moves: ReplayInfoReply, OpenReplayArgs, StartReplayArgs,
│     StartReplayReply, SeekReplayArgs,
│     open_replay, start_replay, pause_replay, resume_replay,
│     seek_replay, stop_replay, replay_control_tx
└── dsp_task.rs               (~350 lines, NEW)
      moves: FFT_SIZE, MIN_EMIT_INTERVAL, MIN_LEVEL_EMIT_INTERVAL,
      PEAK_DECAY_DB_PER_EMIT, AUDIO_CHUNK_SAMPLES,
      spawn_dsp_task, DspTaskCtx + impl, emit_* methods, handle_capture
```

Required signature changes: make `AppState`, `Session`, `SessionSource`, `LiveBits`, `ReplayBits`, `CaptureControl`, `AudioStopInfo`, `IqStopInfo`, `session_poisoned`, `capture_sender`, `radio_snapshot` `pub(crate)` so the three new sibling files can import them. That's it — no new `unsafe`, no new dependencies, no behaviour change.

Net effect: `commands.rs` drops from 1611 → ~520 lines; the DSP worker becomes independently testable once measurement-time mocks are added; future Phase 7 commands land in `capture_cmd.rs` or a new `analysis_cmd.rs` without touching lifecycle code.

### 5.4 CSS cleanup map (deferred to measured pass)

`src/App.css` is 974 lines / 126 selectors. Not inspected in detail this pass. Chromium Coverage (§4.3) will tell us the actual unused percentage. Until then this is a hypothesis, not a finding.

## 6. Correctness findings

- **WAV streaming writer is atomic-by-move, not atomic-by-rename.** `WavStreamWriter::create` opens the *temp* path directly (`src-tauri/src/capture/wav.rs:100-121`) without a `.tmp` + rename. Correct for the capture flow because the temp path *is* disposable — the final destination only receives the fully-finalized file via `finalize_capture` (`src-tauri/src/ipc/commands.rs:886-889`). On a crash mid-record the temp file is left with a zero-size RIFF/data header, which is a dead file — no user-visible bad artifact at the destination. Documenting this invariant in a comment would help (see §10).
- **Bookmarks schema version is written but never validated.** `BookmarksFile { version, bookmarks }` stores `version: u32` (`src-tauri/src/bookmarks.rs:35-39`) and always writes `FILE_VERSION = 1`, but the load path (`src-tauri/src/bookmarks.rs:66-73`) never checks it. If a Phase 7 bump introduces v2 with extra fields, v1 RAIL will parse it with missing fields filled from serde defaults (which silently means losing data on the next save). **Small fix before Phase 7**: refuse to load if `file.version > FILE_VERSION` and surface `RailError::CaptureError("bookmarks.json version N is newer than supported")`. S.
- **`DemodChain::apply(SetBandwidthHz)` rebuilds both the channel filter *and* the mode-dependent pieces.** `reconfigure_channel` (`src-tauri/src/dsp/demod/mod.rs:158-162`) builds fresh taps and calls `self.decim.set_taps(taps)` (preserves the decimator's delay line presumably); `reconfigure_mode` (`src-tauri/src/dsp/demod/mod.rs:164-173`) **replaces** `audio_lpf`, `fm`, `am`, `deemph` — which resets their delay lines. That's correct because the signal chain has changed; but the user will hear one ~few-ms click on mode/bandwidth change. Known acceptable.
- **`id: format!("{:x}", now_nanos.as_nanos())` for bookmarks** (`src-tauri/src/bookmarks.rs:127`) collides under 1 ns resolution. Practically safe because `add` is serialized under a mutex and system nanos are monotonic enough, but brittle if the store ever gains bulk-import. Acceptable for V1.
- **Prefill path is distinct.** `DspInput::Cf32Prefill` bypasses the rate limiter and is capped to `FFT_SIZE` samples (`src-tauri/src/ipc/commands.rs:1466-1494`). Good — it's exactly what the replay backfill needs and can't starve live emits.

## 7. Security / privacy

Threat model (restated): local desktop app, no network features, single-user, developer-distributed binaries. The goal is "no crash surface the user can reach with ordinary input" and "no PII logs".

- **Tauri capabilities** (`src-tauri/capabilities/default.json`): only `core:default`, `dialog:allow-save`, `dialog:allow-open`. Minimal. No filesystem capability — backend mediates all file writes through commands.
- **CSP**: `security.csp: null` (`src-tauri/tauri.conf.json:21`). For a local-only Tauri app that loads only bundled assets, `null` is *functionally* safe, but as a portfolio artifact it's worth switching to an explicit restrictive string such as `"default-src 'self'; img-src 'self' data:; script-src 'self'; style-src 'self' 'unsafe-inline'"`. S.
- **FFI soundness** (`src-tauri/src/hardware/ffi.rs`, `mod.rs`, `stream.rs`): exemplary. See B.5.
- **Filesystem handling**: capture paths are always temp-first (`src-tauri/src/capture/tmp.rs` → `new_tmp_path`) and user-chosen destination via `dialog:allow-save`. `finalize_capture` uses `move_file`. PNG writer uses temp + rename + fsync is not present — `std::fs::rename` is used but without a prior `sync_all` (`src-tauri/src/ipc/commands.rs:961-965`). Bookmarks writer does `sync_all` before rename (`src-tauri/src/bookmarks.rs:88-91`) — good.
- **Path-traversal**: user-chosen destinations go through `tauri-plugin-dialog`, so the OS native file picker validates the path; no string concatenation of user-supplied filenames into paths.
- **Logging PII**: `env_logger::try_init()` at `src-tauri/src/lib.rs:16` with no default. At `info` level the code logs device index+vid:pid+friendly-name (`src-tauri/src/hardware/mod.rs:83`) and disconnect reasons — nothing sensitive, no user paths.
- **Dependency footprint**: `cargo tree -d` duplicates are entirely inside the Tauri/Wry stack. `rail`'s direct deps (`src-tauri/Cargo.toml`) are 10 crates, all mainstream. `npm audit --omit=dev`: 0 vulns. Consider adding `cargo audit` to CI (§8.3).
- **Network**: neither `reqwest` nor `hyper` nor equivalent appears in `Cargo.toml`; `npm ls` shows no `axios`/`fetch` wrappers. App is local-only as documented.

## 8. Tests & CI

### 8.1 Current coverage (summary)

- 36 Rust tests: DSP math (FFT, windows, filters, decimator, resampler), demod chain end-to-end with tones + amplitude + squelch, FM/AM unit, capture WAV header + round-trip, capture SigMF round-trip with meta sidecar, bookmarks, replay path math, iso8601 helper.
- 0 frontend tests; frontend validation relies on `tsc --noEmit` + `vite build` as contract-shape checks.

### 8.2 Proposed small additions (no new test frameworks)

- `src-tauri/src/dsp/demod/mod.rs`: add `reconfigure_roundtrip_preserves_audio` — feed a tone, call `apply(SetBandwidthHz(new))`, confirm the chain does not panic and produces samples of expected order of magnitude.
- `src-tauri/src/ipc/commands.rs` (or after the split, in `dsp_task.rs`): add an isolated test that exercises `DspTaskCtx::emit_waterfall_frames` with a synthetic `DspInput::Cf32Prefill` and asserts the scratch buffer in the follow-up is *reused* (`let cap0 = ...capacity(); ...; assert_eq!(cap_after, cap0)`). This locks in the §4.1 hot-path fix.
- `src-tauri/src/bookmarks.rs`: add a test that parses a synthetic `version: 2` file and expects `RailError::CaptureError` once §6 fix lands.

### 8.3 CI gaps (YAML snippets, text only — do not apply)

Add to `.github/workflows/ci.yml`:

```yaml
      - name: cargo fmt --check
        working-directory: src-tauri
        run: cargo fmt --all -- --check

      - name: cargo test --all-targets
        working-directory: src-tauri
        run: cargo test --all-targets --no-fail-fast

      - name: cargo audit
        working-directory: src-tauri
        run: |
          cargo install --locked cargo-audit
          cargo audit --deny warnings
```

```yaml
  frontend:
    # ...existing...
      - name: npm audit
        run: npm audit --omit=dev --audit-level=high
```

Optional Windows job (the only platform the author personally verifies):

```yaml
  rust-windows:
    name: Rust (Windows build check)
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Fetch librtlsdr Windows prebuilts
        shell: pwsh
        run: |
          Invoke-WebRequest -Uri "https://github.com/rtlsdrblog/rtl-sdr-blog/releases/download/v1.3.6/Release.zip" -OutFile "librtlsdr.zip"
          Expand-Archive "librtlsdr.zip" -DestinationPath "librtlsdr-win"
          New-Item -ItemType Directory -Force -Path vendor/librtlsdr-win-x64 | Out-Null
          Get-ChildItem -Recurse -Path "librtlsdr-win" -Include rtlsdr.dll,rtlsdr.lib |
            ForEach-Object { Copy-Item $_.FullName -Destination vendor/librtlsdr-win-x64/ -Force }
      - name: cargo check
        working-directory: src-tauri
        run: cargo check --all-targets
```

## 9. Docs vs code drift

### 9.1 `docs/ARCHITECTURE.md §3` — commands & events table

The current `docs/ARCHITECTURE.md` lists commands and events that do not match the code. Minimal doc edits required:

| Doc claim | Code reality | Source |
| --- | --- | --- |
| `invoke('set_frequency', { frequencyHz })` | `invoke('retune', { args: { frequencyHz } })` returns `{ frequencyHz }` | `src/ipc/commands.ts:68-69`, `src-tauri/src/ipc/commands.rs:421-459` |
| `invoke('start_stream'): Promise<void>` | `invoke('start_stream', { args, waterfallChannel, audioChannel }): Promise<StartStreamReply>` | `src/ipc/commands.ts:45-54`, `src-tauri/src/ipc/commands.rs:232-319` |
| `invoke('save_iq_clip', { durationMs })` | three-step flow: `start_iq_capture` → `stop_iq_capture` → `finalize_iq_capture`; plus `discard_capture` on cancel | `src-tauri/src/ipc/commands.rs:801-908` |
| Event `waterfall-frame` (named event, float32 ArrayBuffer) | per-session `tauri::ipc::Channel<InvokeResponseBody>` passed to `start_stream` — **no such named event exists** | `src-tauri/src/ipc/commands.rs:3-6,236`, `src/ipc/commands.ts:45-54` |
| Event `audio-chunk` | same — `audioChannel: Channel<ArrayBuffer>` argument, not a named event | same |
| Event `signal-level` | exists, named event, JSON payload, correct in doc | `src-tauri/src/ipc/events.rs:56-70`, `src/ipc/events.ts:32-37` |
| Event `device-status` | exists, named event, correct in doc | `src-tauri/src/ipc/events.rs:24-50` |
| `RailError` variants | code has exactly the 6 variants listed in `ARCHITECTURE.md §6.3` (`DeviceNotFound`, `DeviceOpenFailed`, `StreamError`, `DspError`, `CaptureError`, `InvalidParameter`) — consistent | `src-tauri/src/error.rs:12-30` |

Undocumented but-real events: `replay-position` (`src-tauri/src/ipc/events.rs:21`, `src/ipc/events.ts:10`). Add it to the doc.

Undocumented commands (add to the table): `check_device`, `available_gains`, `set_ppm`, `set_mode`, `set_bandwidth`, `set_squelch`, bookmarks (`list_bookmarks`, `add_bookmark`, `remove_bookmark`, `replace_bookmarks`), capture (`start_audio_capture`, `stop_audio_capture`, `start_iq_capture`, `stop_iq_capture`, `finalize_capture`, `finalize_iq_capture`, `discard_capture`, `screenshot_suggestion`, `save_screenshot`), replay (`open_replay`, `start_replay`, `pause_replay`, `resume_replay`, `seek_replay`, `stop_replay`).

### 9.2 `docs/TIMELINE.md` Phase 6 claims — all verified

- "GitHub Actions CI (cargo build + clippy)" — CI runs clippy `-D warnings`, `cargo test --lib`, `tsc --noEmit`, `vite build`. Verified.
- "All `clippy` warnings resolved" — clippy output is empty (`review-artifacts/clippy.txt`).
- "No `unwrap()` in non-test code" — verified against grep (§5.1).
- "All `/docs/` files reviewed and accurate" — inaccurate for `ARCHITECTURE.md §3` (§9.1); accurate for the rest as far as this pass checked.

### 9.3 Minor doc edits

- `CONTRIBUTING.md:3` typo: `"educatioal"` → `"educational"`.
- `CONTRIBUTING.md:33-34` says CI runs `cargo test --lib` — match the eventual CI upgrade to `--all-targets`.

## 10. Suggested inline comment placeholders

Text-only. The follow-up execution session may apply or discard each.

- `src-tauri/src/ipc/commands.rs:1505` — suggested comment:
  `// TODO(perf): reuse a DspTaskCtx-owned scratch Vec instead of drain(..).collect(); see REVIEW_V1.md §4.1.`
- `src-tauri/src/ipc/commands.rs:1514` — suggested comment:
  `// Vec allocation is forced by Tauri's InvokeResponseBody::Raw(Vec<u8>) API (tauri 2.10). Leave as-is unless upstream adds a borrowed variant.`
- `src-tauri/src/ipc/commands.rs:1537` — suggested comment:
  `// TODO(perf): same as emit_waterfall_frames — reuse audio_tail scratch; see REVIEW_V1.md §4.1.`
- `src-tauri/src/capture/wav.rs:100` — suggested comment:
  `// NOTE: opens the final path directly, not path.tmp. Safe because callers always pass a temp path produced by capture::tmp::new_tmp_path; final placement is done atomically via move_file in ipc::commands::finalize_capture.`
- `src-tauri/src/bookmarks.rs:66` — suggested comment:
  `// TODO(phase-7): reject file.version > FILE_VERSION before Phase 7 bumps the schema. See REVIEW_V1.md §6.`
- `src-tauri/tauri.conf.json:21` — suggested change (text only): replace `"csp": null` with an explicit restrictive CSP such as `"csp": "default-src 'self'; img-src 'self' data:; script-src 'self'; style-src 'self' 'unsafe-inline'"`.
- `docs/ARCHITECTURE.md §3` — suggested section header change: split "Events (Rust → React, streaming)" into "Named events (JSON)" (for `device-status`, `signal-level`, `replay-position`) and "Streaming channels (binary)" (for the per-session `Channel<ArrayBuffer>` passed to `start_stream`/`start_replay`), matching how the code is actually wired.
- `src-tauri/src/ipc/commands.rs:1` — suggested module doc addition: `// NOTE: this file is slated for a move-only split along capture_cmd.rs / replay_cmd.rs / dsp_task.rs — see REVIEW_V1.md §5.3.`
- `CONTRIBUTING.md:3` — suggested edit: fix typo `"educatioal"` → `"educational"`.

## 11. Remediation backlog

### Must-fix (blockers before Phase 7)

- **M-1** Update `docs/ARCHITECTURE.md §3` IPC contract and events table to match the code (§9.1). Otherwise any Phase 7 contributor (including future-you) will wire a listener for events that don't exist. **Effort: S.**
- **M-2** Add Phase 7 schema-guard to `BookmarksStore::load_file` — refuse `version > FILE_VERSION` (§6). Otherwise a v2 bookmark format (likely in Phase 7 annotations work) will silently lose data on round-trip. **Effort: S.**

### Should-fix (cheap wins, land before Phase 7 opens)

- **S-1** Kill per-frame allocations in `emit_waterfall_frames` / `emit_audio_chunks` (§4.1). Two `DspTaskCtx` scratch fields + swap `collect()` for `extend`. Also add the focussed test proposed in §8.2. **Effort: S.**
- **S-2** Split `src-tauri/src/ipc/commands.rs` per §5.3. Move-only PR; no behaviour change. **Effort: S (half a day).**
- **S-3** Expand CI per §8.3: `cargo fmt --check`, `cargo test --all-targets`, `cargo audit`, `npm audit --omit=dev`, optional `windows-latest` check job. **Effort: S.**
- **S-4** Tighten `tauri.conf.json` CSP from `null` to an explicit restrictive string (§7, §10). **Effort: S.**
- **S-5** Fix `CONTRIBUTING.md` typo and align its CI command list with the upgraded workflow. **Effort: S.**

### Nice-to-have

- **N-1** Add a `Web Worker` for the colormap loop in `drawWaterfallRow` (`src/components/Waterfall/index.tsx:314-326`). Only worth it if §4.3 measurement shows this loop consuming ≥1 ms/frame at FFT=2048. Otherwise skip. **Effort: M.**
- **N-2** Share event-name constants between Rust (`src-tauri/src/ipc/events.rs`) and TS (`src/ipc/events.ts`) via a build-time generated file (`include_str!` on Rust side, `import` on TS side through a tiny Vite plugin), or at minimum a shared JSON that both sides read. Only meaningful before the event catalogue grows. **Effort: M.**
- **N-3** CSS coverage pass on `src/App.css` (§5.4). **Effort: M.**
- **N-4** Rust-side perf profiling hook: add `RUST_LOG=trace` histogram at the emit site (behind `#[cfg(feature = "profile")]`) so the follow-up session can capture numbers without touching source for each run. **Effort: M.**

## 12. Open questions carried forward

1. Is Windows the sole personally-verified target, or does the author want to invest in a Windows CI job now? (Bearing on S-3 scope.)
2. Are Phase 7 bookmarks/annotations allowed to reuse the existing `BookmarksStore` format, or should a separate `annotations.json` be created so bookmark schema can stay at v1 forever? (Bearing on M-2 exact implementation.)
3. Does the author want the `waterfall-frame` / `audio-chunk` naming preserved as *pseudonyms* in the docs (for readability) or removed entirely in favour of "streaming channels"? (Bearing on M-1 wording.)
4. Acceptable performance floor: is 25 fps waterfall a *target* or a *cap*? If target, §4.3 experiment 1 gets priority; if cap, skip.
5. Audio latency target: the current 80 ms lookahead + ≤ few hundred ms demod delay is fine for broadcast FM but audible for CW/SSB. Will Phase 7 care?
6. Is there appetite for an offline/"no-device" demo mode that plays back a bundled SigMF sample, to satisfy the portfolio-demo use case on machines without a dongle? Small but adds polish.

### User answer to open questions

1. Keep the app a Windows only app.
2. I like the current bookmark system, please update doc to keep it this way.
3. I prefer ( for all cases ) to update the doc to fit the code to avoid depreceated or misleading naming.
4. I understand the question but dont have an answer , the current result looks good enough.
5. We need to reduce latency as much as possible, drom antenna to the backend and in the UI ( I often have latency between pause/resume ui action and audio stop)
6. You can do that using the file @docs/assets/demo_iq.sigmf-data