# Contributing to RAIL

RAIL is a solo educational project ( I am educating myself to SIGINT ), but contributions and bug reports are
welcome.

## Ground rules

1. Read [`CLAUDE.md`](CLAUDE.md) before touching code — it lists the
   non-negotiable architectural and style rules.
2. Read the relevant file under [`docs/`](docs/) before changing code
   that touches DSP, hardware, IPC, or capture formats. Start from
   [`docs/README.md`](docs/README.md).
3. Keep Rust and frontend concerns on their own side of the IPC boundary.
   No DSP in React, no UI logic in Rust.

## Development loop

```bash
npm install
npm run tauri dev
```

Platform prerequisites (librtlsdr, Zadig on Windows, etc.) are covered in
the root `README.md`.

**IPC event names:** Named Tauri events (JSON bus) are listed in
[`shared/ipc_event_names.json`](shared/ipc_event_names.json). `npm run build`
runs codegen for [`src/ipc/generated/eventNames.ts`](src/ipc/generated/eventNames.ts);
Rust picks up the same file via `src-tauri/build.rs`. Edit the JSON (or run
`node scripts/gen-ipc-event-names.mjs`)—do not hand-edit the generated TS.

**Optional backend emit profiling:** `cargo build --features profile` with
`RAIL_PROFILE=1` and `RUST_LOG=rail_perf=info` — see [`docs/PERF.md`](docs/PERF.md).

## Before opening a PR

The following must all pass locally:

```bash
# Rust
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets --no-fail-fast
cd ..

# Frontend
npx tsc --noEmit
npm run build
npm audit --omit=dev --audit-level=high
```

GitHub Actions runs the same checks on every push and PR — see
`.github/workflows/ci.yml`.

## Style

- **Rust**: zero `clippy` warnings, no `unwrap()` or `expect()` outside
  tests, public functions carry `///` doc comments. See
  [`docs/CONVENTIONS.md`](docs/CONVENTIONS.md).
- **TypeScript**: no `any`, functional components only.
- **Commits**: conventional-style prefixes (`feat`, `fix`, `docs`,
  `refactor`, `test`, `chore`) with a short scope, e.g.
  `fix(hardware): handle device disconnect during stream`. Each commit
  must leave the project buildable.

## Reporting bugs

Open a GitHub issue with:
- OS + version
- RTL-SDR model (e.g. NESDR Smart v5, generic R820T2)
- Exact steps to reproduce
- Relevant log output from `cargo tauri dev` (set `RUST_LOG=debug` for
  more detail)

For bugs involving specific signals, include the center frequency, sample
rate, and mode — and, if possible, attach a short SigMF capture using
RAIL's "Save IQ clip" button.

## Scope

RAIL's scope is defined in [`docs/PRD.md`](docs/PRD.md). V1 targets
local RTL-SDR listening, FM/AM demodulation, and structured capture.
Features outside that scope (TX, network SDR, digital protocol decoders)
are out of scope for V1 and V1.1.
