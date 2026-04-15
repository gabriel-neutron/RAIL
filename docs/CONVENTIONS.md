# CONVENTIONS.md — Code Style and Project Conventions

## 1. Rust conventions

### Naming
- Types and structs: `PascalCase`
- Functions and variables: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Modules: `snake_case`

### Error handling
- No `unwrap()` or `expect()` outside of tests
- All public functions return `Result<T, RailError>`
- Use `?` operator for propagation
- Log errors with context before returning: `eprintln!("context: {err}")`

### Documentation comments
- All public functions: `///` doc comment explaining purpose
- Parameters that are non-obvious: document units (Hz, dB, samples)
- If function implements math from `/docs/DSP.md`: add `/// See: DSP.md §N`
- Do not re-explain the math — only reference it

### Module structure
- One responsibility per module
- Keep files under 300 lines — split if longer
- `mod.rs` only re-exports, never implements

### Example
```rust
/// Sets the RTL-SDR center frequency.
/// `freq_hz`: center frequency in Hz (500_000 to 1_750_000_000).
/// See: HARDWARE.md §4 for tuning constraints.
pub fn set_center_freq(dev: &Device, freq_hz: u32) -> Result<(), RailError> {
    // ...
}
```

---

## 2. TypeScript / React conventions

### Naming
- Components: `PascalCase` (filename matches component name)
- Hooks: `useCamelCase`
- Store slices: `camelCase`
- IPC wrappers: match Rust command name in camelCase

### Component rules
- Functional components only
- Props interface defined inline above the component
- No inline styles — use CSS modules or Tailwind classes
- Self-explanatory naming: no comments needed for UI logic

### State management
- All radio state in `store/radio.ts` (zustand)
- All session/capture state in `store/session.ts`
- No local state for data that affects multiple components

### IPC rules
- All `invoke()` and `listen()` calls in `/src/ipc/` only
- Components import from `ipc/`, never call Tauri directly
- All IPC functions are typed — no `any`

---

## 3. File and folder structure rules

- New Rust module → new folder with `mod.rs` under `src-tauri/src/`
- New React component → new folder under `src/components/`
- Component folder contains: `index.tsx`, `styles.module.css` (if needed)
- No barrel files (`index.ts` re-exporting everything) — import directly

---

## 4. Git conventions

- Commits: `<type>(<scope>): <description>` (conventional commits)
- Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`
- Examples:
  - `feat(dsp): implement FM demodulation`
  - `fix(hardware): handle device disconnect during stream`
  - `docs(dsp): add phase wrap edge case to DSP.md`
- No commits with "WIP" or "fix stuff" — be specific
- Each commit should leave the project in a buildable state

---

## 5. What not to do

- No `TODO` comments without a linked issue or `// TODO: see DSP.md §N`
- No commented-out code in commits
- No magic numbers — use named constants with units in the name
  - Bad: `let size = 2048`
  - Good: `const FFT_SIZE_SAMPLES: usize = 2048`
- No frontend logic in Rust, no DSP logic in React
- No logging with `println!` in production paths — use `log` crate
