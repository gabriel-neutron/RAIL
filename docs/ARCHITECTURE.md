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

RAIL is a **Tauri v2** desktop application.

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
│  └── ipc/         (commands)    │
└────────────┬────────────────────┘
             │ librtlsdr (FFI)
┌────────────▼────────────────────┐
│         RTL-SDR Hardware        │
└─────────────────────────────────┘
```

---

## 2. Module boundaries

### Rust — `/src-tauri/src/`

```
hardware/
  mod.rs          ← RTL-SDR open/close/configure
  stream.rs       ← IQ sample reader, ring buffer
dsp/
  mod.rs
  fft.rs          ← FFT pipeline (rustfft wrapper)
  waterfall.rs    ← magnitude, dB, FFT shift
  demod/
    fm.rs         ← FM demodulation (owned)
    am.rs         ← AM demodulation (owned)
    ssb.rs        ← SSB stub (V1.1)
  filter.rs       ← decimation, window functions
capture/
  mod.rs
  sigmf.rs        ← SigMF read/write
  session.rs      ← session metadata
ipc/
  commands.rs     ← Tauri command handlers
  events.rs       ← binary event emitters
```

### React — `/src/`

```
components/
  Waterfall.tsx       ← canvas rendering, colormap
  FrequencyControl/   ← frequency input, step buttons
  ModeSelector/       ← CW/AM/FM/USB/LSB buttons
  FilterControl/      ← bandwidth controls
  SignalMeter/        ← dBm display
  AudioControls/      ← volume, mute, record
  CapturePanel/       ← session list, export
store/
  radio.ts            ← zustand store (tuned freq, mode, gain)
  session.ts          ← capture session state
hooks/
  useWaterfall.ts     ← binary event listener → canvas
  useAudio.ts         ← PCM stream → Web Audio API
ipc/
  commands.ts         ← typed wrappers for Tauri commands
  events.ts           ← typed wrappers for Tauri events
```

---

## 3. Tauri IPC contract

### Commands (React → Rust, request/response)

```typescript
// Tune the radio
invoke('set_frequency', { frequencyHz: number }): Promise<void>

// Change demodulation mode
invoke('set_mode', { mode: 'FM' | 'AM' | 'USB' | 'LSB' | 'CW' }): Promise<void>

// Set filter bandwidth
invoke('set_bandwidth', { bandwidthHz: number }): Promise<void>

// Set gain (dB, 0 = auto)
invoke('set_gain', { gainDb: number }): Promise<void>

// Start/stop streaming
invoke('start_stream'): Promise<void>
invoke('stop_stream'): Promise<void>

// Capture controls
invoke('start_recording'): Promise<void>
invoke('stop_recording'): Promise<{ filePath: string }>
invoke('save_iq_clip', { durationMs: number }): Promise<{ filePath: string }>
```

### Events (Rust → React, streaming)

```typescript
// Waterfall frame — binary float32 array of length N (magnitude in dB)
// Event name: 'waterfall-frame'
// Payload: ArrayBuffer (float32, N elements)

// Audio chunk — binary float32 array (PCM samples, mono, 44100 Hz)
// Event name: 'audio-chunk'
// Payload: ArrayBuffer (float32)

// Signal meter update — current and peak dBm
// Event name: 'signal-level'
// Payload: { current: number, peak: number }

// Device status
// Event name: 'device-status'
// Payload: { connected: boolean, error?: string }
```

**Binary event format**: Tauri v2 supports raw `Vec<u8>` payloads.
Cast float32 array as bytes: `bytearray = float32_slice.as_bytes()`.
Frontend receives as `ArrayBuffer`, wraps with `new Float32Array(buffer)`.

---

## 4. Threading model

```
Main thread (Tauri)
  └── Command handlers (async, tokio)

tokio runtime
  ├── Stream task      ← reads IQ from RTL-SDR in a loop
  │     └── sends IQ chunks to DSP channel (mpsc)
  ├── DSP task         ← reads IQ, runs FFT + demod
  │     ├── emits waterfall-frame event
  │     └── sends PCM to audio channel
  └── Audio task       ← buffers PCM, emits audio-chunk event
```

**IQ buffer**: `std::sync::mpsc` or `tokio::sync::mpsc` channel between
stream task and DSP task. Ring buffer size: 8 frames minimum to absorb
USB jitter without dropping samples.

**Never block the main thread** — all hardware and DSP work is async/tokio.

---

## 5. Data flow diagrams

### Waterfall frame

```
RTL-SDR USB callback
  → raw IQ bytes (u8 pairs) → convert to f32 complex (I/255-0.5, Q/255-0.5)
  → ring buffer (capacity: 8×N)
  → FFT task reads N samples
  → apply Hann window (see DSP.md §7)
  → rustfft forward FFT
  → compute magnitude and dB (see DSP.md §2)
  → FFT shift (swap halves)
  → emit binary Tauri event 'waterfall-frame'
  → React Float32Array → colormap → canvas pixel row → scroll down
```

### Audio path

```
DSP task → demodulated f32 samples
  → decimate to 44100 Hz
  → emit binary Tauri event 'audio-chunk'
  → React Web Audio API AudioContext
  → AudioBuffer → AudioBufferSourceNode → speakers
```

---

## 6. Error handling strategy

### Rust
- All public functions return `Result<T, RailError>`
- Define `RailError` enum in `src-tauri/src/error.rs`
- Hardware errors (device not found, USB drop) → emit `device-status` event with error
- DSP errors (buffer underrun) → log warning, skip frame, do not crash
- File I/O errors → return Err to frontend via command response

### React
- All `invoke()` calls wrapped in try/catch
- Hardware disconnection → show modal, offer reconnect
- Audio buffer underrun → log only, do not show error to user
- Never silently swallow errors — log with context

### RailError variants (minimum)
```rust
pub enum RailError {
    DeviceNotFound,
    DeviceOpenFailed(String),
    StreamError(String),
    DspError(String),
    CaptureError(String),
    InvalidParameter(String),
}
```
