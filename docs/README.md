# RAIL — Documentation Index

> This is the single source of truth for all project documentation.
> Before creating or modifying any `/docs/` file, read this index.
> Before writing math or physics in code comments, check if it belongs here.

---

## Index

| File | Covers | Status |
|---|---|---|
| `DSP.md` | FFT, demodulation math, filter theory, waterfall pipeline | Active |
| `ARCHITECTURE.md` | Tauri IPC, data flow, module boundaries, threading model | Active |
| `HARDWARE.md` | RTL-SDR specifics, librtlsdr, sampling rates, gain, tuning | Active |
| `SIGNALS.md` | Signal types, SigMF format, capture schema, frequency domains | Active |
| `CONVENTIONS.md` | Code style, naming, error handling, file structure | Active |
| `TECH_STACK.md` | Approved technologies, versions, dependencies, tooling choices | Active |
| `PRD.md` | Product requirements, scope, UX goals, acceptance criteria | Active |
| `TIMELINE.md` | Development phases, milestones, task ordering | Active |

---

## Rules (enforced by hook)

Every time a file in `/docs/` is created or modified:

1. Is this topic already covered in an existing file? → Add there, don't create new
2. Is the new file listed in this README? → Add it before committing
3. Does this file exceed ~150 lines? → Add a table of contents at the top
4. Is math being duplicated from `DSP.md`? → Remove duplication, add a reference instead
5. Is this README still under 200 lines? → Keep it an index, not a content file

---

## Ownership map

> Use this to find the right file for any topic.

- **"How does FM demodulation work?"** → `DSP.md`
- **"What sample rate should I use?"** → `HARDWARE.md`
- **"How does Rust talk to React?"** → `ARCHITECTURE.md`
- **"What is SigMF format?"** → `SIGNALS.md`
- **"How should I name this function?"** → `CONVENTIONS.md`
- **"What tech and versions are approved?"** → `TECH_STACK.md`
- **"What are the product goals and acceptance criteria?"** → `PRD.md`
- **"What should I build next?"** → `TIMELINE.md`
- **"What FFT size should I use?"** → `DSP.md`
- **"How do I handle RTL-SDR errors?"** → `HARDWARE.md`
