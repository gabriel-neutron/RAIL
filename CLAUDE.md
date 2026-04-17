# CLAUDE.md — RAIL Project Rules

> This file defines core rules for Claude Code. Read it fully before touching any file.
> For domain knowledge (DSP, hardware, architecture), consult `/docs/README.md` first.

---

## Project identity

RAIL (Radio Analysis and Intel Lab) is a Tauri desktop application.
- **Rust backend**: hardware access, DSP, signal processing
- **React frontend**: UI only — waterfall rendering, controls, display
- **No business logic in the frontend**

---

## Non-negotiable rules

### Code
- Rust: follow `clippy` with no warnings allowed
- React: functional components only, no class components
- No `unwrap()` in Rust except in tests — use `Result` and propagate errors
- No `any` in TypeScript
- All public Rust functions must have doc comments
- Frontend functions must be self-explanatory — clear naming over comments

### Documentation
- Before modifying any file in `/docs/`: read the **Documentation rules** section below
- Never duplicate content between code comments and `/docs/` files
- Backend functions referencing math must cite the relevant `/docs/` file and section

### Architecture
- IPC: Rust → React via **binary Tauri events** (float32 ArrayBuffer) for streaming data
- IPC: React → Rust via **Tauri commands** for control (tune, mode, stop)
- Audio: Rust outputs PCM → Web Audio API plays it
- No direct hardware calls from frontend, ever

---

## Documentation rules

> These rules apply every time Claude Code creates or modifies a file in `/docs/`.

1. Check `/docs/README.md` — does a file already cover this topic?
2. If yes: add to the existing file, do not create a new one
3. `/docs/README.md` must stay under 200 lines
4. Every new `/docs/` file must be added to `/docs/README.md` immediately
5. Every `/docs/` file longer than ~150 lines must start with a table of contents (max 50 lines)
6. Do not duplicate math or physics explanations that already exist in `/docs/DSP.md` or `/docs/SIGNALS.md`
7. Backend code comments must reference docs, not re-explain them

---

## Libraries — approved list

| Purpose | Library | Notes |
|---|---|---|
| FFT | `rustfft` | Do not reimplement FFT |
| RTL-SDR control | `rtlsdr-rs` or raw FFI via `libloading` | Phase 1+. Direct binding, no rtl_tcp daemon |
| USB enumeration | `nusb` | Phase 0 device detection (pure Rust, no FFI) |
| Serialization | `serde` + `serde_json` | Standard |
| IPC binary | Tauri binary events | float32 ArrayBuffer |
| Frontend state | `zustand` | No Redux |
| Frontend canvas | native Canvas API | No third-party waterfall libs |
| Capture format | SigMF (`.sigmf-meta` + `.sigmf-data`) | See `/docs/SIGNALS.md` |

---

## What Claude Code must not do

- Do not implement FFT from scratch
- Do not call `rtl_tcp` — use direct librtlsdr binding
- Do not add frontend libraries without checking this file first
- Do not write DSP math in comments — link to `/docs/DSP.md`
- Do not generate placeholder or mock data silently — flag it explicitly
- Do not modify `/docs/` files without following documentation rules above

---

## When stuck on DSP or physics

1. Stop
2. Read `/docs/DSP.md` and `/docs/SIGNALS.md`
3. If the answer is not there, add it to the correct `/docs/` file before implementing
4. Flag any math that cannot be verified from docs as `// TODO: verify math — see DSP.md`
