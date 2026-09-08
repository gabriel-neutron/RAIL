# ARCHITECTURE.md — System Design Reference

## Table of contents
1. [Overview](#1-overview)
2. [Module boundaries](#2-module-boundaries)
3. [Tauri IPC contract](#3-tauri-ipc-contract)
4. [Threading model](#4-threading-model)
5. [Data flow diagrams](#5-data-flow-diagrams)
6. [Error handling strategy](#6-error-handling-strategy)

---

## 1. Overview

RAIL is a **Tauri v2** desktop application. React owns the UI, Rust owns hardware, DSP, and file I/O. The two sides exchange structured requests over Tauri commands and high-rate binary frames over per-session channels.

```
┌─────────────────────────────────┐
│         React Frontend          │  ← UI only, no DSP, no hardware
│  (TypeScript + Canvas API)      │
└────────────┬────────────────────┘
             │ Tauri IPC
┌────────────▼────────────────────┐
│         Rust Backend            │  ← hardware, DSP, file I/O
│  ├── hardware/    (RTL-SDR)     │
│  ├── dsp/         (FFT, demod)  │
│  ├── capture/     (SigMF I/O)   │
│  ├── replay.rs    (SigMF play)  │
│  └── ipc/         (commands)    │
└────────────┬────────────────────┘
             │ librtlsdr (FFI)
┌────────────▼────────────────────┐
│         RTL-SDR Hardware        │
└─────────────────────────────────┘
```

### 1.1 Offline demo

The app ships with a short sample IQ capture at [`docs/assets/demo_iq.sigmf-data`](assets/demo_iq.sigmf-data) (+ its `.sigmf-meta` sidecar). On a host without a dongle, feed that path to the standard replay flow — `open_replay` followed by `start_replay` — and the waterfall, spectrum, and audio paths behave exactly as for a live stream.

---

## 2. Module boundaries

### Rust — `/src-tauri/src/`

```
hardware/      mod.rs, stream.rs, tuner.rs, ffi.rs
dsp/           input.rs, fft.rs, waterfall.rs, filter.rs, demod/{mod,fm,am}.rs
decoders/      mod.rs, adsb.rs, aprs.rs, rds.rs, pocsag.rs  (Phase 17)
capture/       sigmf.rs, wav.rs, tmp.rs
replay.rs      SigMF playback reader
scanner.rs     Wideband sweep engine (sequential retune → dwell → power measure)
bookmarks.rs   versioned JSON store (atomic write)
ipc/           commands.rs, events.rs
error.rs       RailError (serde-tagged)
```

### React — `/src/`

```
components/    Waterfall, FrequencyControl, ModeSelector, FilterBandMarker,
               SignalMeter, AudioControls, Transport, MenuBar, PpmControl,
               Scanner (band-activity canvas + sweep controls)
store/         zustand: radio / capture / replay / scanner
hooks/         useWaterfall, useAudio, useResizeTick
viewport/      spectrumViewport, cellAxis, canvasSizing, formatHz — the pure
               bin/Hz/pixel conversions and labels every overlay draws
               against (DSP.md §9)
ipc/           transport.ts (port), tauriTransport.ts (the one Tauri-importing
               module), commands.ts, radioControl.ts (control seam), events.ts
test/          mockTransport.ts, fakeClock.ts — the adapters tests inject
```

### The frontend control seam

Nothing in `store/` or `components/` imports `@tauri-apps` at runtime. The
host is reached through the `IpcTransport` port in `ipc/transport.ts`
(`invoke` plus the two native path pickers); `main.tsx` installs
`tauriTransport` at start-up, and a test installs `createMockTransport()`.
An unset transport throws, so a missed wiring fails loudly.

Everything between "the user moved a knob" and "a command left the app"
lives in `ipc/radioControl.ts`: per-verb debouncing over an injected
`Clock`, the gain and PPM clamping policy, and the two dispatch guards.
The seam holds no session state — it takes `canControl` (streaming) and
`canTouchHardware` (streaming and not replaying) as predicates and reads
them at dispatch time, so it cannot desync from the stores. The store
builds its dispatcher inside the zustand creator, which keeps the debounce
timers out of module scope and resettable between tests. Components keep
their `disabled` props: the guards decide what is sent, not what is shown.

Command wrappers follow one envelope rule, mirroring the Rust signatures:
a command whose handler takes a `#[derive(Deserialize)]` args struct is
passed a single `{ args }` object, and a parameterless command is passed
nothing. Sibling `Channel` handles ride alongside `args`, not inside it.

The port also mints those channels (`createChannel(onMessage)`), which is
why `startScan(args, onFrame)` takes a frame callback rather than a
channel: the handler is bound at construction, so no frame can land
before something is listening. The scan session that consumes those
frames lives in `store/scanner.ts` (`runScanSession` / `cancelScanSession`),
the single place the scan wire format is decoded.

---

## 3. Tauri IPC contract

RAIL uses two distinct IPC surfaces: **named JSON events** (low-rate status and transport updates) and **per-session binary channels** (high-rate waterfall and audio frames). There are no `waterfall-frame` or `audio-chunk` named events — all high-rate traffic is channel-based.

### 3.1 Commands (React → Rust, request/response)

| Command | Wrapper | Purpose |
| --- | --- | --- |
| `ping` | `ping()` | Liveness probe |
| `check_device` | `checkDevice()` | Enumerate the RTL-SDR (index, name) |
| `start_stream` | `startStream(args, waterfallCh, audioCh)` | Open device, start DSP worker; returns FFT/sample-rate metadata |
| `stop_stream` | `stopStream()` | Cancel the read thread, join the DSP worker |
| `set_gain` | `setGain({ auto, tenthsDb? })` | Auto or explicit gain step |
| `retune` | `retune(frequencyHz)` | Retune the tuner; echoes applied frequency |
| `set_ppm` | `setPpm(ppm)` | Tuner PPM correction |
| `set_mode` | `setMode(mode)` | `FM` / `NFM` / `AM` / `USB` / `LSB` / `CW` |
| `set_bandwidth` | `setBandwidth(bandwidthHz)` | Rebuild the channel filter |
| `set_squelch` | `setSquelch(thresholdDbfs \| null)` | Audio-gate threshold |
| Bookmarks | `listBookmarks`, `addBookmark`, `removeBookmark`, `replaceBookmarks` | Versioned JSON store CRUD |
| Capture | `start/stopAudioCapture`, `start/stopIqCapture`, `finalizeCapture`, `finalizeIqCapture`, `discardCapture` | Stage-then-finalize file I/O |
| Screenshot | `screenshotSuggestion`, `saveScreenshot` | Suggest filename, atomic PNG write |
| Replay | `openReplay`, `startReplay`, `replayTransport({kind})` | Transport for SigMF captures (incl. [`docs/assets/demo_iq.sigmf-data`](assets/demo_iq.sigmf-data)); `kind` is `play` / `pause` / `seek`. Teardown goes through `stopStream` |
| Scanner | `startScan(args, onFrame)`, `stopScan()` | Sequential frequency sweep; `startScan` returns ordered `frequenciesHz[]`; one f32 per step on `scanCh` |

### 3.2 Named events (Rust → React, JSON)

Payload shapes are not repeated here — they live in
[`shared/ipc_events.json`](../shared/ipc_events.json), the single contract
`scripts/gen-ipc-events.mjs` compiles into both
[`src-tauri/src/ipc/generated/events.rs`](../src-tauri/src/ipc/generated/events.rs)
and [`src/ipc/generated/events.ts`](../src/ipc/generated/events.ts). Field
names are snake_case in the contract and camelCase on the wire. Only the
cadence, which the contract does not model, is listed below.

| Event | Cadence |
| --- | --- |
| `device-status` | On connect / disconnect / error |
| `signal-level` | ≤ 25 Hz, rate-limited with peak decay |
| `signal-classification` | ~2 Hz while streaming |
| `replay-position` | ~25 Hz while replay is open |
| `scan-step` | Per scanner retune (~200–240 ms cadence during a sweep); keeps display components in sync |
| `scan-complete` | Once, when a full sweep finishes without hitting squelch |
| `scan-stopped` | Once, when the scanner halts early on a detected signal |

Decoder events planned for Phases 17–22 (`docs/TIMELINE.md`), each landing as
one contract entry alongside its producer:

| Event | Payload | Cadence |
| --- | --- | --- |
| `adsb-1090-frame` | `{ icao, lat?, lon?, altFt?, callsign?, speedKts?, headingDeg?, rawHex }` | Per decoded Mode S DF17 frame; rate-limited ≤ 10 fps |
| `aprs-packet` | `{ fromCallsign, to, lat?, lon?, comment, rawInfo }` | Per valid AX.25 APRS frame; rate-limited |
| `rds-group` | `{ piCode, groupType, psName?, radioText?, programmeType, trafficProgramme }` | Per complete RDS group (PS name emitted when all 8 chars assembled) |
| `pocsag-message` | `{ capcode, function, content, baudRate }` | Per POCSAG message frame after BCH error correction |

Every payload emits through the one [`Emit`](../src-tauri/src/ipc/event_contract.rs)
blanket impl; the frontend subscribes through the one `subscribeIpcEvent` in
[`src/ipc/events.ts`](../src/ipc/events.ts).

### 3.3 Streaming channels (Rust → React, binary)

High-rate frames travel on `tauri::ipc::Channel<InvokeResponseBody>` opened by the frontend and passed as command arguments:

- **`waterfallChannel`** (`start_stream`, `start_replay`): `FFT_SIZE × 4 = 32768` bytes of little-endian `f32` magnitude (dB), at ≤ 25 fps.
- **`audioChannel`** (same): `AUDIO_CHUNK_SAMPLES × 4 ≈ 7 KB` of mono `f32` PCM at 44.1 kHz.
- **`scanChannel`** (`start_scan`): two little-endian `f32` (8 bytes) per frequency step — `signal_avg_db` (average power in the target channel window) then `noise_floor_db` (median of the full spectrum); the frontend takes the difference as SNR. Exactly one message per frequency, including steps whose retune failed (both fields `-inf`), so step order matches `frequenciesHz[]` from the command reply. Frames are positional only — they carry no step index — so the frontend buffers any frame that arrives before `start_scan` resolves and drains it in order once `frequenciesHz[]` is known; dropping an early frame would shear every later result by one step.

Rust sends `InvokeResponseBody::Raw(Vec<u8>)`; the frontend receives an `ArrayBuffer` and wraps it with `new Float32Array(buffer)` (see [`src/hooks/useWaterfall.ts`](../src/hooks/useWaterfall.ts) and [`src/hooks/useAudio.ts`](../src/hooks/useAudio.ts)).

---

## 4. Threading model

```
Main thread (Tauri async)
  └── Command handlers (tokio)

Dedicated std::thread (per stream)
  └── hardware/stream.rs: rtlsdr_read_async loop
        └── bounded mpsc<DspInput>(cap=8), try_send + drop counter

tokio::task::spawn_blocking (per stream)
  └── DSP worker (ipc/commands.rs: DspTaskCtx)
        ├── fs/4 shift + channel filter + demod chain
        ├── decoder side-chain (decoders/ — Phase 17)
        │     runs after demod chain; emits typed JSON events
        ├── waterfall emit on waterfallChannel  (≤ 40 ms cadence)
        ├── signal-level emit  (≤ 40 ms cadence, peak decay)
        └── audio emit on audioChannel          (per AUDIO_CHUNK_SAMPLES)
```

The read thread is `std::thread` (not tokio) because `rtlsdr_read_async` blocks until cancelled. The DSP worker is `spawn_blocking` because work is CPU-bound; it uses `blocking_recv` on the IQ channel. Stop is explicit and idempotent: `stop_stream` cancels the scanner task and **awaits** it, removes the `Session`, then awaits the read and DSP handles; `IqStream::Drop` cancels+joins as a safety net. Awaiting the scanner first is what makes the tuner sound: the sweep reaches the device through the `Tuner` port (`hardware/tuner.rs`), whose `RtlSdrTuner` adapter holds a borrowed, non-owning device pointer. The reader thread remains the device's owner, and still conditionally `mem::forget`s it when the dongle disappears mid-stream (see `hardware/stream.rs`).

Replay mirrors this shape — a tokio task reads the SigMF file, decodes samples, and feeds the same DSP worker type via a `DspInput::Cf32Prefill` priming variant.

---

## 5. Data flow diagrams

### 5.1 Waterfall

```
RTL-SDR USB callback (librtlsdr thread)
  → hardware/stream.rs on_iq: slice.to_vec() + try_send(DspInput::RtlU8)
  → DspTaskCtx::run (spawn_blocking)
      ├── iq_u8_to_complex → shifted scratch (reused)
      ├── apply_fs4_shift (in-place)
      └── emit_waterfall_frames
            ├── fft_pending.extend (amortized)
            └── while ≥ FFT_SIZE:
                  process_shifted → FFT → |·|² → 10·log10 → fft-shift (in place)
                  waterfallChannel.send(Raw(bytes))
  → useWaterfall.onmessage: push(new Float32Array(buffer)) into pending
  → rAF drain (≤ 360 frames/tick) → Waterfall component → canvas row
```

### 5.2 Audio

```
DspTaskCtx → DemodChain (channel filter → decim → mode → LPF → resample 44.1 kHz)
  → emit_audio_chunks: audioChannel.send(Raw(bytes)) per AUDIO_CHUNK_SAMPLES
  → useAudio: AudioBuffer + AudioBufferSourceNode with ~80 ms lookahead
  → GainNode (volume/mute) → destination
```

### 5.3 Capture and replay

Capture writes to a temp path produced by `capture::tmp::new_tmp_path`, then `finalize_capture` / `finalize_iq_capture` atomically moves the file(s) to the user-chosen destination. On cancel, `discard_capture` deletes the temp files.

Replay reuses the same DSP worker; `open_replay` parses the SigMF meta, `start_replay` spawns the reader, and `replay-position` ticks the transport slider in the UI.

### 5.4 Decoder pipeline (Phase 17)

```
DspTaskCtx — after chain.process(), runs emit_decoder_frames():
  ├── AdsB1090Decoder  (center_hz ≈ 1090 MHz)
  │     IQ magnitude → Mode S CRC-24 → DF17 → "adsb-1090-frame"
  ├── AprsDecoder      (center_hz ≈ 144.390/144.800 MHz)
  │     NFM audio → Bell 202 → AX.25 → "aprs-packet"
  ├── RdsDecoder       (center_hz in 87.5–108 MHz, mode=FM)
  │     WBFM baseband → 57 kHz BPSK → "rds-group"
  └── PocsagDecoder    (center_hz in 152–159 / 929–931 MHz)
        NFM audio → FSK → BCH(31,21) → "pocsag-message"
```

See `docs/DECODERS.md` for framing details, frequency gating, and error handling.

---

## 6. Error handling strategy

### Rust

- Public functions return `Result<T, RailError>` with the six variants in [`src-tauri/src/error.rs`](../src-tauri/src/error.rs): `DeviceNotFound`, `DeviceOpenFailed`, `StreamError`, `DspError`, `CaptureError`, `InvalidParameter`.
- `RailError` serializes as `{ kind, message }` (serde `tag = "kind", content = "message"`); TS mirror in [`src/ipc/commands.ts`](../src/ipc/commands.ts).
- Hardware disconnects emit `device-status` with `connected: false`.
- DSP-side frame drops (channel full) are counted and logged at power-of-two thresholds — never panic on backpressure.
- Mutex poisoning is funneled through `session_poisoned` for a single error path.

### React

- Every `invoke()` is wrapped; failures surface as toasts and/or inline error states.
- Audio underruns are non-fatal — log only, keep scheduling.
- Errors are never silently swallowed; callers log with context.
