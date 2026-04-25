# RAIL — Multi-Perspective Code & Portfolio Review

> Generated 2026-04-25. Three agents reviewed the full codebase from different standpoints.
> No code changes were made. All findings are advisory.

---

## Reviewers

| Agent | Persona | Focus |
|---|---|---|
| **SIGINT Engineer** | Senior SIGINT/DSP engineer, 15+ yrs Rust/C++ | Technical correctness, DSP credibility, architecture |
| **Intel HR Reviewer** | Hiring manager, OSINT/SIGINT/defense roles | Portfolio value, credibility, presentation |
| **SDR Hobbyist** | Experienced RTL-SDR user, SDR#/GQRX/SDR++ veteran | Usability, setup, practical gaps, missing features |

---

## 1 · SIGINT Engineer Review

### Top 5 Issues

**1. FFT normalization is wrong — all dB readings are ~6 dB low.**
File: `src-tauri/src/dsp/fft.rs` lines 71–76. The Hann window is applied before the FFT but the window's coherent gain (~0.5 for Hann) is not compensated. The normalizer divides by `n`, which cancels the DFT normalization but not the window's amplitude loss. The net error is approximately −6 dB on every displayed value. This systematically corrupts the classifier's SNR thresholds (set at 10 dB / 20 dB) and the scanner's dBFS readout. Fix: replace `self.norm = n as f32` with `self.norm = window.iter().sum::<f32>()`.

**2. Single-stage 65-tap FIR decimator — aliasing not suppressed for WBFM.**
File: `src-tauri/src/dsp/filter.rs` lines 33–65; `demod/mod.rs` line 33. Decimating from 2.048 MHz by 8 requires the anti-alias filter to attenuate everything above 128 kHz (the fold-over frequency). For a 100 kHz WBFM passband the transition band is only 28 kHz, requiring ~240 taps for −40 dB stopband. The 65-tap design will produce audible aliasing intermodulation on WBFM. Multi-stage decimation (e.g., 2×4 with proper per-stage filter design) is the standard fix.

**3. SSB Hilbert phasing: audio LPF introduces uncompensated delay on one path.**
File: `src-tauri/src/dsp/demod/ssb.rs` lines 67–78; `demod/mod.rs` downstream. The I and Q paths are correctly aligned through the 129-tap Hilbert FIR (64-sample group delay compensated on both paths). However, the 65-tap audio LPF applied after the demodulator adds ~32 samples of group delay to only one path, reintroducing phase error in the reconstructed waveform. Result: harmonic distortion on voice SSB.

**4. IPC binary channel has no framing header — fragile against coalesced messages.**
File: `src-tauri/src/ipc/dsp_task.rs` lines 467–471; `src/hooks/useWaterfall.ts` line 98. Waterfall frames are raw `Vec<u8>` (little-endian f32) with no length prefix, version byte, or sentinel. If Tauri ever coalesces two messages, or if the frame format changes (adding a timestamp), the frontend will silently misinterpret data with no detectable error. A 6-byte header (`[0xFF, type:u8, frame_count:u32]`) would prevent silent corruption and enable forward compatibility.

**5. `AtomicU32` for center frequency + `Relaxed` ordering — race condition with classifier.**
File: `src-tauri/src/ipc/dsp_task.rs` line 389. The center frequency is written from the retune path and read in the DSP/classifier path using `Ordering::Relaxed` with no synchronization fence. A retune can leave the classifier reading `center_hz` for the new frequency while processing a spectrum snapshot collected at the old frequency, producing spurious mode confirmations during retuning. Low-probability but non-zero; use `Ordering::SeqCst` or coordinate via the existing channel message.

### Top 5 Improvements

1. **Window coherent gain compensation in `fft.rs`** — immediately fixes all dB readings and classifier SNR thresholds.
2. **Multi-stage polyphase decimation** — replaces single 65-tap FIR; a 256-tap polyphase at first stage costs 32 MACs/output sample vs. the current 65 and achieves proper stopband rejection.
3. **DC-blocking IIR before SSB demodulator** — a 2-pole high-pass at ~10 Hz on the complex baseband after decimation eliminates I/Q imbalance DC bias before it reaches the Hilbert phasing path.
4. **Spectral flatness as classifier discriminator** — geometric/arithmetic mean power ratio distinguishes NFM voice from 9600-baud APRS (same bandwidth, similar envelope variance) and DMR/D-STAR from NFM voice at 70 cm.
5. **Framed IPC format with manifest packet** — emit a JSON manifest before the first data frame, then frame all subsequent payloads with a 6-byte header. Enables frame drop detection, FFT size changes without restart, and replay metadata injection.

### Verdict

RAIL is architecturally disciplined for a hobby SDR tool — the threading model, FFI safety annotations, error propagation, and IPC boundary are noticeably above average. The classifier phasing-method test and full pipeline round-trip are genuine regression tests most projects never write. However, the FFT normalization error means every dB value in the app is systematically wrong by ~6 dB, which corrupts the classifier and scanner in the field. The single-stage 65-tap decimator will produce audible WBFM aliasing. A senior DSP engineer would need both fixed, plus a framed IPC format, before calling this production-quality signal processing.

---

## 2 · Intelligence HR Reviewer

### Top 5 Issues

**1. README undersells the project for the target audience.**
The opening line reads "This is an educational build: I wanted to go past 'press play in someone else's app.'" For a defense or intelligence hiring manager, that is pre-emptive apology. The technically credible work — hand-written librtlsdr FFI, from-scratch Hilbert FIR phasing, three-path classifier with frequency priors, binary IPC channels, SigMF-compliant capture — does not appear above the fold. A recruiter who bounces after the first paragraph walks away thinking "SDR hobby project."

**2. Zero field-validated classifier output documented.**
SIGNALS.md §5.4 honestly documents that classifier thresholds (`env_var=0.15`, `asym=15dB`) were set analytically from synthetic IQ and have not been validated against real RF. That honesty is correct engineering practice and the right thing to document. In a portfolio context, it is a live confession that the feature most directly mapping to SIGINT work has never been demonstrated on real signals. There is exactly one screenshot in the repo (the README waterfall) and no evidence the classifier has ever correctly labeled a real transmission.

**3. No tests outside the classifier and SSB demodulator.**
`classifier.rs` has 15 tests, `ssb.rs` has 2. There are no `#[cfg(test)]` blocks in `dsp/fft.rs`, `dsp/demod/fm.rs`, `dsp/demod/am.rs`, `capture/sigmf.rs`, `scanner.rs`, or the bookmark store. CI runs `cargo test --all-targets` but is largely testing two files. For a role involving signal integrity and traceable analysis pipelines, a sparse test surface is a flag — especially when the classifier tests show the candidate knows exactly how to write good unit tests.

**4. The intelligence angle is implicit rather than declared.**
SIGNALS.md documents 18 signal types, a band taxonomy from 87 MHz to L-band, AIS/ADS-B/APRS classification priors, and a frequency-prior-first dispatch architecture. That is genuine SIGINT domain knowledge. The README never uses the word "SIGINT" or "signal intelligence." A non-technical HR screener has no way to know this project required understanding of ATC communications, AIS transponder protocols, or VHF maritime band planning.

**5. No downloadable release artifact — the project cannot be experienced without a full dev setup.**
TIMELINE.md Phase 7 exit criteria include a downloadable `.exe` installer at a release tag. A `release.yml` workflow file exists. But no release tag is visible and no installer is available. A hiring manager who wants to run the app cannot. The SigMF demo file ships with the repo and the offline demo mode works — but only someone who has Rust, Node.js, and librtlsdr installed can launch it.

### Top 5 Improvements

1. **Rewrite the README opening** — drop "educational build" framing; lead with capabilities and technical proof points; put a screenshot above the fold; add a one-paragraph "Intelligence value" section.
2. **Add a "Field results" section** — point the antenna at FM broadcast, ATC (AM), maritime VHF or ISM 433; capture one screenshot per signal type showing the classifier badge, waterfall, and frequency readout; put in `docs/assets/field/`.
3. **Add DSP unit tests for FM and FFT pipeline** — apply the same test discipline as the classifier to `dsp/fft.rs` (known-frequency input → expected peak bin) and `dsp/demod/fm.rs` (constant-phase-deviation IQ → expected audio amplitude).
4. **Publish a real GitHub release with Windows installer** — execute the TIMELINE.md Phase 7 exit criterion: `git tag v0.1.0`, push the tag, let `release.yml` build. This makes the project downloadable without a dev environment.
5. **Add a "Signals intelligence design note"** — 200–300 words in SIGNALS.md explaining why the frequency prior dominates, why the asymmetry threshold is 15 dB, and what the next analytical step would be (per-peak dwell, protocol decoder integration, TDOA if multiple receivers). This signals analytical tradecraft, not just implementation.

### Verdict

RAIL is more technically serious than it presents itself. The FFI, Hilbert phasing demodulator, three-path classifier, binary IPC architecture, and SigMF-standard capture pipeline are credible artifacts demonstrating real signal processing knowledge. The documentation is unusually thorough for a personal project — the ARCHITECTURE.md IPC contract table is more precise than most professional repos. The problem is presentation and verification: the README sounds like a hobby project, no field validation of the SIGINT layer is documented, and no runnable artifact exists for download. For a technically literate panel doing a deep review this clears the bar; for the HR screening round — eight minutes, cannot build a Rust project — it probably does not.

---

## 3 · SDR Hobbyist Review

### Top 5 Issues

**1. Windows setup will stop most first-time users cold.**
The README points to `docs/HARDWARE.md §6` for Windows driver setup. For a user who has never used Zadig, that sentence is a cliff. They must: install Zadig, select the right USB device, switch to WinUSB (not libusb-win32), then find a matching prebuilt DLL and place it in `vendor/librtlsdr-win-x64/`. None of this is guided in the app. When it fails, the StatusPill says "No RTL-SDR device found" with no actionable suggestion. SDR# ships a driver installer and a one-page PDF; RAIL gives a paragraph and an external doc link.

**2. Squelch control is not exposed in the UI despite being fully implemented in the backend.**
`set_squelch`, `DemodControl::SetSquelchDbfs`, the squelch gate in `DemodChain::process`, and even mode-switching rescaling in the radio store are all present. But in `App.tsx` there is no squelch widget rendered in the control panel. The `setSquelchDbfs` action in the store has no component calling it for live receive. Squelch is the first thing any amateur radio listener reaches for on NFM — its absence is immediately noticeable and practically crippling for monitoring use.

**3. Scanner is too slow for practical band-hunting and will miss short bursts.**
The scanner retuning loop: hardware settle 40 ms + dwell 50 ms + scheduling jitter = ~90 ms minimum per step. A 20 MHz sweep at 200 kHz steps = 100 steps × 90 ms = 9–15 seconds per pass. Worse, it polls a single `latest_dbfs_bits` atomic — a single instantaneous RMS snapshot per dwell. Transmissions shorter than one DSP frame (~20 ms) will be missed entirely. PMR446, ISM 433 burst devices, and pager traffic commonly have sub-20 ms bursts. SDR# and comparable tools use instantaneous FFT bandwidth; the 2 MHz of spectrum the dongle sees simultaneously is not being used during the scan.

**4. Signal classification is an interesting prototype that will frustrate intermediate users.**
The frequency prior table is sparse: FM broadcast, aviation, maritime, 2m/70cm amateur, NOAA-APT, AIS, APRS, ISM 433. NOAA weather radio (162.4–162.55 MHz), FRS/GMRS (462–467 MHz), public safety UHF (450–470 MHz), MURS, ACARS — common North American scan targets — return empty candidates. The single-prior path deliberately skips spectral analysis, so the "confirmed" green badge means "the frequency database says this band carries NFM," not "the signal is actually NFM." An intermediate user who tests this quickly finds it confidently labels anything in a known band regardless of what is actually there.

**5. Bookmarks save frequency and name but not mode or bandwidth.**
Tuning to a maritime bookmark at 156.8 MHz does not switch to NFM 12.5 kHz. The user must manually re-select mode and bandwidth on every recall. The FilterControl bandwidth presets for WBFM include 150 kHz, which is not a standard broadcast FM allocation; canonical WBFM channels are 200 kHz, and the practical narrowband step is 75 kHz. AM presets (`6k, 8k, 10k`) lack a 2.7 kHz option useful for shortwave SSB that arrives as AM.

### Top 5 Improvements

1. **Add squelch slider to the control panel** — the entire backend stack exists; a horizontal slider between AudioControls and PpmControl, range −100 to 0 dBFS with a "disabled" position, would make NFM monitoring immediately usable. ~30-minute frontend addition.
2. **Extend bookmarks to store mode, bandwidth, and optionally PPM** — apply them on tune (treat missing fields as "no change"). Turns a frequency list into an actual receiver channel bank.
3. **Replace polling-based scanner measurement with full-dwell FFT accumulation** — instead of polling `latest_dbfs_bits`, have the DSP task accumulate `max_dbfs_per_bin` over the dwell window and return a float array. Catches burst traffic; enables the band-activity canvas to show a mini-spectrum bar per step rather than a single dot.
4. **Add commonly scanned North American and European bands to frequency prior** — NOAA weather radio (162.4–162.55 MHz), FRS/GMRS (462–467 MHz), MURS (151–154 MHz), public safety UHF (450–470 MHz), ACARS/VDL (129.125 MHz), DAB III (174–240 MHz). Each addition is 2–3 lines in the classifier match arm.
5. **Add DC offset annotation to waterfall** — the fs/4 LO offset correctly moves the hardware DC spike; label it. Add `DC offset: ±{sampleRate/4} MHz` to the waterfall status bar, and a thin annotation line on the FrequencyAxis at `center_hz ± sample_rate/4`. Eliminates the most common "what is that vertical line?" beginner question.

### Verdict

For a complete beginner: not yet recommended. Windows setup is too rough, squelch is missing from the UI, and there is no in-app guidance when setup fails. For an intermediate hobbyist who already has drivers working: worth running. The code is unusually clean for an SDR side project, DC offset handling is correct and tested, the demodulator chain covers the practical modes, and the SigMF capture/replay workflow is more thoughtful than most comparable tools. The gap between "promising project" and "tool I'd recommend to someone who just bought their first RTL-SDR" is about four additions: squelch slider, mode-aware bookmarks, better band coverage, and a setup guide that doesn't require reading external documentation.

---

## 4 · Cross-Agent Recurring Problems

These problems were identified independently by two or more reviewers — they are the highest-confidence issues.

| # | Problem | Identified by |
|---|---|---|
| **A** | FFT dB readings systematically wrong (~6 dB low due to missing window gain compensation) | SIGINT engineer + hobbyist (scanner accuracy) |
| **B** | No squelch UI despite complete backend implementation | Hobbyist (explicit) + HR (feature gap) |
| **C** | Classifier thresholds unvalidated on real hardware; "confirmed" badge misleading | HR + hobbyist |
| **D** | No downloadable release — project cannot be experienced without full dev stack | HR (explicit) + hobbyist (setup friction) |
| **E** | README undersells the project; technical feats and intelligence relevance invisible | HR (explicit) + hobbyist (setup section) |
| **F** | Scanner misses burst traffic — single-poll measurement is inadequate | Hobbyist (explicit) + SIGINT (IPC/accuracy) |
| **G** | Test coverage gaps — DSP chain (FM, FFT, SigMF round-trip) has no tests | HR + SIGINT engineer |
| **H** | Classifier frequency prior table too sparse for practical use | HR + hobbyist |

---

## 5 · Prioritized Action List

### P0 — Critical bugs (fix before any public release)

| Item | File(s) | Effort |
|---|---|---|
| **Fix FFT window coherent gain compensation** — replace `self.norm = n as f32` with `self.norm = window.iter().sum()`. Add a unit test asserting a full-scale complex tone peaks at 0 ±0.5 dBFS. | `dsp/fft.rs` | 1–2 h |

### P1 — Quick wins (high impact, low effort)

| Item | File(s) | Effort |
|---|---|---|
| **Add squelch slider to control panel** — the full backend stack (command, store action, mode rescaling) already exists; add only the UI widget | `App.tsx`, new `SquelchControl` component | ~2 h |
| **Publish GitHub release v0.1.0** — `git tag v0.1.0`, push tag, let `release.yml` produce the Windows `.exe` installer. The SigMF demo file ships with the app so reviewers can test without hardware | `release.yml`, git tag | ~1 h |
| **Rewrite README first two paragraphs** — drop "educational build" framing; lead with capabilities, tech proof points, and a screenshot above the fold; add "Intelligence value" one-paragraph section | `README.md` | ~1 h |
| **Extend bookmarks to store mode + bandwidth** — add optional `mode` and `bandwidth_hz` fields to `Bookmark`; apply on tune (missing = no change) | `bookmarks.rs`, `ipc/commands.rs`, `store/bookmarks.ts` | ~3 h |
| **Add DC offset annotation to waterfall status bar** — add `DC: ±{sampleRate/4} MHz` to the existing status bar and a thin annotation line on `FrequencyAxis` | `Waterfall.tsx`, `FrequencyAxis.tsx` | ~1 h |

### P2 — Medium effort, high value

| Item | File(s) | Effort |
|---|---|---|
| **Add field-validation screenshots and "Field results" section** — capture classifier badges on FM broadcast, ATC/AM, maritime VHF; put in `docs/assets/field/`; add to README | `README.md`, `docs/assets/` | ~2–4 h (needs hardware session) |
| **Expand classifier frequency prior** — add NOAA weather radio, FRS/GMRS, MURS, public safety UHF, ACARS, DAB III | `classifier.rs` | ~2 h |
| **Add DSP unit tests for FM demodulator and FFT pipeline** — constant-phase-deviation IQ → expected audio amplitude; known-frequency complex tone → expected peak bin | `dsp/fft.rs`, `dsp/demod/fm.rs` | ~3–4 h |
| **Add "Signals intelligence design note" to SIGNALS.md** — 200–300 words on classifier design rationale, threshold choices, and next analytical steps | `docs/SIGNALS.md` | ~1–2 h |
| **Fix scanner measurement to accumulate max-per-bin over dwell window** — replace `latest_dbfs_bits` single-poll with a float array covering the full dwell; enables burst detection and a richer band-activity canvas | `scanner.rs`, `ipc/dsp_task.rs` | ~4–6 h |

### P3 — Larger improvements (worth doing, but not quick wins)

| Item | Rationale | Effort |
|---|---|---|
| **Multi-stage polyphase decimation** for WBFM anti-alias filter | Single 65-tap FIR has insufficient stopband for 8× decimation; audible WBFM aliasing will follow | ~8–12 h |
| **Add DC-blocking IIR before SSB demodulator** | RTL-SDR I/Q imbalance introduces DC offset that leaks into SSB/CW audio as low-frequency bias | ~2–3 h |
| **Compensate audio LPF group delay in SSB demodulator** | 65-tap LPF downstream of the Hilbert phasing method misaligns I/Q paths by ~32 samples, adding harmonic distortion | ~2–4 h |
| **Add spectral flatness to classifier** | Distinguishes NFM voice from 9600-baud APRS, and DMR/D-STAR from NFM voice at 70 cm | ~4–6 h |
| **Add framed IPC format with manifest packet** | No practical impact today under Tauri v2 guarantees, but enables future format evolution and drop detection | ~4–6 h |
| **In-app Windows setup wizard** | First-run experience for Zadig/DLL installation to unblock beginners | ~1–2 days |

---

## 6 · What to Avoid or Deprioritize

- **Do not add `AtomicU64` for center frequency yet** — RTL-SDR tops out at ~1.7 GHz; the `u32` truncation is a latent bug but not a real one. Fix the ordering issue (`Relaxed` → coordinated update) before worrying about the width.
- **Do not add a third-party UI component library** — the custom UI is a demonstrable technical skill; introducing Material-UI or Chakra would erase that signal.
- **Do not add AI-based classification yet** — the existing heuristic classifier is the right scope for v1; a TensorFlow/Burn model would expand scope significantly and is not needed to prove competence.
- **Do not over-document what is already correctly implemented** — the ARCHITECTURE.md and DSP.md are already unusually good; adding more internal docs will not move the portfolio needle. Field results and a rewritten README will.
- **Do not address the IPC framing header as a P0** — it is a correctness concern but Tauri v2 channel semantics make practical coalescing unlikely; the FFT normalization bug is the only true P0.
- **Do not rewrite the scanner architecture before fixing the FFT normalization** — the scanner's threshold behavior is currently based on wrong dB values; fixing the normalizer first establishes the correct baseline, then the scanner numbers become meaningful.

---

## Summary

RAIL is a technically serious project presented as a hobby experiment. The architecture is clean, the Rust code quality is high, and the IPC design is more disciplined than most comparable tools. The critical problems fall into three buckets:

1. **One DSP correctness bug** (FFT normalization, P0) that corrupts every dB reading in the app.
2. **Several missing-but-already-built UI features** (squelch slider, mode-aware bookmarks) that are straightforward frontend additions.
3. **A presentation gap** (README, no release, no field screenshots) that hides genuine technical work from the audience most likely to evaluate it.

Fixing P0 and P1 items would take roughly one focused weekend and would substantially improve both the technical correctness of the app and its credibility as a portfolio artifact.
