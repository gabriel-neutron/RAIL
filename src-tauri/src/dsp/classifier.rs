//! Signal classification heuristics.
//!
//! Implements the three-path classifier defined in `docs/SIGNALS.md §5`:
//! frequency prior dominates; spectral analysis only runs when the prior is
//! ambiguous (≥ 2 candidates) or absent (unknown band).
//! DSP primitives (FFT magnitude, IQ envelope) are defined in
//! `docs/DSP.md §2–5`. No math is derived here — only cited.
//!
//! ## Output model
//!
//! [`ClassificationResult`] has two independent fields:
//!
//! - `confirmed`: a single mode wire-name, populated only when SNR ≥ 20 dB.
//!   Maps to a **green** ModeSelector button.
//! - `candidates`: mode wire-names from the frequency prior, always populated
//!   for known bands regardless of signal strength. Maps to **yellow** buttons.
//!
//! The two fields are independent — a confirmed mode may or may not appear
//! in candidates. The frontend is responsible for deduplication.
//!
//! ## Known limitations
//!
//! The envelope variance (0.15) and sideband asymmetry (15 dB) thresholds
//! were set analytically from synthetic IQ data and have not been validated
//! across diverse hardware dongles, antenna configurations, or propagation
//! conditions. Real-world field testing is needed before these values are
//! considered stable. See `docs/SIGNALS.md §5.4` (TODO note).
// TODO: field-validate classifier thresholds (env_var=0.15, asym=15dB) —
// see docs/SIGNALS.md §5.4 for context.

use num_complex::Complex;

// ── Public types ──────────────────────────────────────────────────────────────

/// Signal label string. Values match the taxonomy in `docs/SIGNALS.md §3`.
pub type Label = &'static str;

/// Mode wire-names that map to buttons in the ModeSelector.
/// Subset of the full label taxonomy — labels with no demodulatable mode
/// (OOK, AIS, ADS-B, …) are not in this set.
pub type WireName = &'static str;

pub const LABEL_WBFM: Label = "WBFM";
pub const LABEL_NBFM: Label = "NBFM";
pub const LABEL_AM: Label = "AM";
pub const LABEL_USB: Label = "USB";
pub const LABEL_LSB: Label = "LSB";
pub const LABEL_CW: Label = "CW";
pub const LABEL_OOK: Label = "OOK";
pub const LABEL_AIS: Label = "AIS";
pub const LABEL_APRS: Label = "APRS";
pub const LABEL_NOAA_APT: Label = "NOAA-APT";
pub const LABEL_ADS_B: Label = "ADS-B";
pub const LABEL_DIGITAL_NARROWBAND: Label = "digital_narrowband";
pub const LABEL_DIGITAL_WIDEBAND: Label = "digital_wideband";

/// Classifier output.
///
/// - `confirmed` — spectrally confirmed mode wire-name (green button). `None`
///   when SNR is too low or the identified signal type has no selectable mode.
/// - `candidates` — mode wire-names from frequency prior (yellow buttons).
///   Always populated for known bands; empty otherwise.
/// - `reason` — human-readable reason for `confirmed`, shown as tooltip.
#[derive(Debug, Clone)]
pub struct ClassificationResult {
    pub confirmed: Option<WireName>,
    pub candidates: Vec<WireName>,
    pub reason: String,
}

impl ClassificationResult {
    fn no_signal(candidates: Vec<WireName>) -> Self {
        Self { confirmed: None, candidates, reason: String::new() }
    }
}

/// Minimum SNR (dB above noise floor) for a peak to trigger spectral analysis.
/// Below this the signal is indistinguishable from noise artefacts.
/// Ref: docs/SIGNALS.md §5.1.
const MIN_PEAK_SNR_DB: f32 = 10.0;

/// Minimum SNR required to populate `confirmed`. 20 dB provides enough margin
/// above the extreme-value noise peak (≈15 dB above median for 8192 bins) so
/// that background noise never cycles the green indicator.
const MIN_CONFIRM_SNR_DB: f32 = 20.0;

// ── Public API ────────────────────────────────────────────────────────────────

/// Classify the current signal from a dB-scaled FFT spectrum and IQ samples.
///
/// `spectrum` is FFT-shifted (DC at center index `n/2`), exactly as produced
/// by [`crate::dsp::fft::FftProcessor::process`] — see `docs/DSP.md §2`.
/// `center_hz` is the tuner centre frequency in Hz.
///
/// Returns a [`ClassificationResult`] where:
/// - `confirmed` is populated only when a real signal is clearly detected.
/// - `candidates` is always populated for known frequency bands.
///
/// ## Decision architecture (docs/SIGNALS.md §5.3)
///
/// The frequency prior is the primary source of truth. Spectral analysis
/// only runs when the prior is ambiguous (≥ 2 candidates) or absent:
///
/// - **0 candidates** (unknown band): broad spectral classify — WBFM/NBFM/AM only;
///   SSB/CW are never guessed without an explicit frequency prior.
/// - **1 candidate** (e.g. FM 88–108 MHz, aviation 108–137, maritime): trust the
///   prior directly; no spectral analysis performed — eliminates false cycling.
/// - **≥ 2 candidates** (e.g. 2 m amateur: NFM/USB/CW): spectral measurements
///   disambiguate within the prior's candidate set.
pub fn classify(
    spectrum: &[f32],
    iq: &[Complex<f32>],
    sample_rate_hz: u32,
    center_hz: u64,
) -> ClassificationResult {
    // Frequency prior is independent of SNR — compute it first so it is
    // always returned even when no signal is present.
    let candidates = frequency_prior_candidates(center_hz);

    if spectrum.is_empty() || iq.is_empty() {
        return ClassificationResult::no_signal(candidates);
    }

    let n = spectrum.len();
    let hz_per_bin = sample_rate_hz as f32 / n as f32;
    let dc_center = n / 2;
    let dc_guard: usize = 10;

    // Step 1 — noise floor (median of all non-DC bins). Ref: docs/SIGNALS.md §5.1.
    let noise_floor = estimate_noise_floor(spectrum, dc_center, dc_guard);

    // Step 2 — peak: highest bin > noise_floor + MIN_PEAK_SNR_DB, outside DC guard.
    // Ref: docs/SIGNALS.md §5.1.
    let peak = spectrum
        .iter()
        .enumerate()
        .filter(|(i, _)| ((*i as isize) - dc_center as isize).unsigned_abs() > dc_guard)
        .filter(|(_, &db)| db > noise_floor + MIN_PEAK_SNR_DB)
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap());

    let Some((peak_bin, &peak_db)) = peak else {
        return ClassificationResult::no_signal(candidates);
    };

    let snr = peak_db - noise_floor;

    // Step 3 — occupied bandwidth: walk outward until power < noise_floor + 6 dB.
    // Using –3 dB from peak would be falsely narrow for WBFM (carrier >> sidebands).
    // Ref: docs/SIGNALS.md §5.1, docs/DSP.md §2.
    let threshold_db = noise_floor + 6.0;
    let mut lo = peak_bin;
    let mut hi = peak_bin;
    while lo > 0 && spectrum[lo - 1] > threshold_db {
        lo -= 1;
    }
    while hi + 1 < n && spectrum[hi + 1] > threshold_db {
        hi += 1;
    }
    let bw_hz = (hi - lo + 1) as f32 * hz_per_bin;
    let bw_family = BwFamily::from_hz(bw_hz);

    // Step 4 — envelope variance (AM vs FM). Ref: docs/SIGNALS.md §5.2.
    let env_var = envelope_variance(iq);
    let is_am_family = env_var > 0.15;

    // Step 5 — sideband asymmetry. Only computed for multi-candidate bands
    // to avoid running a noisy measurement on every single-prior emission.
    let asym_db_opt: Option<f32> = if candidates.len() >= 2 {
        Some(sideband_asymmetry(spectrum, lo, hi, dc_center))
    } else {
        None
    };

    // Step 6 — three-path confirmation dispatch. Ref: docs/SIGNALS.md §5.3.
    let confirmed = if snr >= MIN_CONFIRM_SNR_DB {
        match candidates.len() {
            // Unknown band: broad categories only — no SSB/CW without a prior.
            0 => broad_classify(bw_family, is_am_family),
            // Single-prior band: trust the prior directly, no spectral analysis.
            1 => Some(candidates[0]),
            // Multi-candidate band: spectral picks within the prior's set.
            _ => pick_from_candidates(&candidates, bw_family, is_am_family,
                                      asym_db_opt.unwrap()),
        }
    } else {
        None
    };

    let reason = if confirmed.is_some() {
        build_reason(bw_hz, env_var, asym_db_opt.unwrap_or(0.0), center_hz, snr)
    } else {
        String::new()
    };

    ClassificationResult { confirmed, candidates, reason }
}

// ── BW family ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum BwFamily {
    Wideband,
    Narrowband,
    Voice,
    Narrow,
}

impl BwFamily {
    fn from_hz(bw_hz: f32) -> Self {
        if bw_hz > 150_000.0 {
            Self::Wideband
        } else if bw_hz > 25_000.0 {
            Self::Narrowband
        } else if bw_hz > 3_000.0 {
            Self::Voice
        } else {
            Self::Narrow
        }
    }
}

// ── DSP measurements ──────────────────────────────────────────────────────────

/// Median of all non-DC bins. The median is a robust noise-floor estimator:
/// in a spectrum where a signal occupies ≤50 % of bins, the median sits in
/// the noise region and tracks the true noise level. The bottom-20% average
/// previously used was 5–10 dB *below* the true floor, inflating apparent SNR
/// and causing noise peaks to falsely trigger confirmation.
fn estimate_noise_floor(spectrum: &[f32], dc_center: usize, dc_guard: usize) -> f32 {
    let mut bins: Vec<f32> = spectrum
        .iter()
        .enumerate()
        .filter(|(i, _)| ((*i as isize) - dc_center as isize).unsigned_abs() > dc_guard)
        .map(|(_, &v)| v)
        .collect();
    if bins.is_empty() {
        return -100.0;
    }
    bins.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    bins[bins.len() / 2]
}

/// Normalised envelope variance — `Var(|s|) / E[|s|]²`.
/// FM: near zero. AM: > 0.15 threshold. See `docs/SIGNALS.md §5.2`.
fn envelope_variance(iq: &[Complex<f32>]) -> f32 {
    if iq.len() < 2 {
        return 0.0;
    }
    let envelopes: Vec<f32> = iq.iter().map(|s| s.norm()).collect();
    let mean = envelopes.iter().copied().sum::<f32>() / envelopes.len() as f32;
    if mean == 0.0 {
        return 0.0;
    }
    let variance = envelopes.iter().map(|&e| (e - mean).powi(2)).sum::<f32>()
        / envelopes.len() as f32;
    variance / (mean * mean)
}

/// Upper-vs-lower sideband power ratio in dB. SSB: |ratio| > 10 dB.
/// See `docs/SIGNALS.md §5.2`.
fn sideband_asymmetry(spectrum: &[f32], lo: usize, hi: usize, dc_center: usize) -> f32 {
    let mid = (lo + hi) / 2;
    let lower_end = mid.min(dc_center);
    let upper_start = mid.max(dc_center);
    let power_db = |slice: &[f32]| {
        if slice.is_empty() {
            return -200.0_f32;
        }
        let linear: f32 = slice.iter().map(|&db| 10.0_f32.powf(db / 10.0)).sum();
        if linear > 0.0 { 10.0 * linear.log10() } else { -200.0 }
    };
    let lower_power = if lo < lower_end { power_db(&spectrum[lo..lower_end]) } else { -200.0 };
    let upper_power = if upper_start <= hi { power_db(&spectrum[upper_start..=hi]) } else { -200.0 };
    upper_power - lower_power
}

// ── Spectral helpers (used only by the multi-candidate and unknown-band paths) ─

/// Disambiguate within a known multi-candidate set using spectral measurements.
///
/// Called only when `frequency_prior_candidates` returned ≥ 2 results
/// (e.g. 2 m amateur: NFM / USB / CW). Ordered first-match: the first rule
/// that fires wins. Asymmetry threshold raised to 15 dB (vs a naïve 10 dB)
/// to tolerate short-window measurement noise. Ref: docs/SIGNALS.md §5.2–5.3.
fn pick_from_candidates(
    candidates: &[WireName],
    bw_family: BwFamily,
    is_am_family: bool,
    asym_db: f32,
) -> Option<WireName> {
    let has = |name: WireName| candidates.contains(&name);

    if has("FM") && bw_family == BwFamily::Wideband { return Some("FM"); }
    if has("AM") && is_am_family                    { return Some("AM"); }
    if has("USB") && asym_db > 15.0                 { return Some("USB"); }
    if has("LSB") && asym_db < -15.0               { return Some("LSB"); }
    if has("CW") && bw_family == BwFamily::Narrow   { return Some("CW"); }
    if has("NFM")                                   { return Some("NFM"); }
    candidates.first().copied()
}

/// Broad classification for signals at frequencies with no frequency prior.
///
/// Intentionally conservative: only emits wide FM, narrowband FM, or AM.
/// SSB and CW are never guessed without an explicit band prior — attempting
/// to do so from 4 ms of IQ produces unreliable asymmetry measurements.
/// Ref: docs/SIGNALS.md §5.3.
fn broad_classify(bw_family: BwFamily, is_am_family: bool) -> Option<WireName> {
    match bw_family {
        BwFamily::Wideband => {
            if !is_am_family { Some("FM") } else { None }
        }
        BwFamily::Narrowband | BwFamily::Voice => {
            if is_am_family { Some("AM") } else { Some("NFM") }
        }
        // Narrow signals (<3 kHz) could be CW, a data tone, or noise.
        // Without a frequency prior there is no reliable basis to pick one.
        BwFamily::Narrow => None,
    }
}

fn build_reason(bw_hz: f32, env_var: f32, asym_db: f32, center_hz: u64, snr: f32) -> String {
    format!(
        "BW={:.0}kHz, var={:.3}, asym={:.1}dB, SNR={:.1}dB @ {:.3}MHz",
        bw_hz / 1000.0,
        env_var,
        asym_db,
        snr,
        center_hz as f64 / 1e6,
    )
}

// ── Frequency prior ───────────────────────────────────────────────────────────

/// Return the mode wire-names most likely at `center_hz` per the band table
/// in `docs/SIGNALS.md §5.3`. Independent of signal strength — always valid
/// for the tuned frequency. Empty slice for unknown bands.
fn frequency_prior_candidates(center_hz: u64) -> Vec<WireName> {
    let f = center_hz;

    // Spot frequencies
    if hz_near(f, 137_100_000, 5_000) || hz_near(f, 137_620_000, 5_000) {
        // NOAA-APT: FM-family narrow; NFM is the closest demodulatable mode
        return vec!["NFM"];
    }
    if hz_near(f, 129_125_000, 25_000) {
        // ACARS (VHF datalink, DSB-AM): primary North American frequency
        return vec!["AM"];
    }
    if hz_near(f, 144_800_000, 10_000) {
        // APRS (digital AFSK on 2m): NFM carrier, but also USB used nearby
        return vec!["NFM"];
    }
    if hz_near(f, 161_975_000, 5_000) || hz_near(f, 162_025_000, 5_000) {
        // AIS (GMSK): NFM is the nearest demodulatable mode
        return vec!["NFM"];
    }
    if hz_near(f, 433_920_000, 200_000) {
        // ISM 433: OOK bursts — no clean demodulatable mode, but AM is closest
        return vec!["AM", "NFM"];
    }
    if hz_near(f, 1_090_000_000, 500_000) {
        // ADS-B: no usable audio mode
        return vec![];
    }

    // Band ranges — broader, lower priority
    if hz_in(f, 87_500_000, 108_000_000) {
        return vec!["FM"];
    }
    if hz_in(f, 108_000_000, 137_000_000) {
        // VOR/ILS + aviation voice: all AM
        return vec!["AM"];
    }
    if hz_in(f, 144_000_000, 146_000_000) {
        // 2m amateur: FM repeaters, SSB DX, CW — all three are common
        return vec!["NFM", "USB", "CW"];
    }
    if hz_in(f, 151_000_000, 154_000_000) {
        // MURS (Multi-Use Radio Service): unlicensed NFM, 12.5 kHz channels
        return vec!["NFM"];
    }
    if hz_in(f, 162_400_000, 162_551_000) {
        // NOAA weather radio: 7 broadcast frequencies 162.400–162.550 MHz
        return vec!["NFM"];
    }
    if hz_in(f, 156_000_000, 174_000_000) {
        // Maritime VHF: ITU Ch 1–88 including coast guard, AIS
        return vec!["NFM"];
    }
    if hz_in(f, 174_000_000, 240_000_000) {
        // DAB III (Digital Audio Broadcasting Band III): OFDM digital multiplex,
        // no decodable audio mode in RAIL
        return vec![];
    }
    if hz_in(f, 430_000_000, 440_000_000) {
        // 70cm amateur: similar mix to 2m but no CW calling freq defined here
        return vec!["NFM", "USB"];
    }
    if hz_near(f, 446_000_000, 100_000) {
        return vec!["NFM"];
    }
    if hz_in(f, 450_000_000, 470_000_000) {
        // Public safety UHF + FRS/GMRS (462–467 MHz): all NFM
        // Covers analog conventional, P25 Phase 1, trunked, and FRS/GMRS simplex
        return vec!["NFM"];
    }

    vec![]
}

/// True when `freq` is within `tolerance_hz` of `center`.
#[inline]
fn hz_near(freq: u64, center: u64, tolerance_hz: u64) -> bool {
    freq.abs_diff(center) <= tolerance_hz
}

/// True when `freq` is in `[lo_hz, hi_hz)`.
#[inline]
fn hz_in(freq: u64, lo_hz: u64, hi_hz: u64) -> bool {
    freq >= lo_hz && freq < hi_hz
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn make_spectrum(n: usize, center_bin: usize, width_bins: usize, peak_db: f32, floor_db: f32) -> Vec<f32> {
        let mut spec = vec![floor_db; n];
        let lo = center_bin.saturating_sub(width_bins / 2);
        let hi = (center_bin + width_bins / 2 + 1).min(n);
        for s in spec[lo..hi].iter_mut() {
            *s = peak_db;
        }
        spec
    }

    fn fm_iq(n: usize) -> Vec<Complex<f32>> {
        (0..n)
            .map(|i| {
                let phase = 2.0 * PI * 0.1 * i as f32;
                Complex::new(phase.cos(), phase.sin())
            })
            .collect()
    }

    fn am_iq(n: usize) -> Vec<Complex<f32>> {
        (0..n)
            .map(|i| {
                let carrier = 2.0 * PI * 0.1 * i as f32;
                let amplitude = 1.0 + 0.8 * (2.0 * PI * 0.01 * i as f32).sin();
                Complex::new(amplitude * carrier.cos(), amplitude * carrier.sin())
            })
            .collect()
    }

    #[test]
    fn noise_only_returns_no_confirmed() {
        let n = 8192;
        let spectrum = vec![-100.0_f32; n];
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 98_000_000);
        assert!(result.confirmed.is_none());
    }

    #[test]
    fn noise_at_fm_band_still_gives_candidates() {
        let n = 8192;
        let spectrum = vec![-100.0_f32; n];
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 98_000_000);
        assert!(result.confirmed.is_none());
        assert!(result.candidates.contains(&"FM"), "FM band prior expected");
    }

    #[test]
    fn wbfm_at_fm_broadcast_band_confirms_fm() {
        // 200 kHz wide peak, 80 dB SNR at 98 MHz.
        let n = 8192;
        let dc = n / 2;
        let spectrum = make_spectrum(n, dc, 800, -20.0, -100.0);
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 98_000_000);
        assert_eq!(result.confirmed, Some("FM"));
        assert!(result.candidates.contains(&"FM"));
    }

    #[test]
    fn low_snr_signal_has_no_confirmed() {
        // SNR = 12 dB — above MIN_PEAK_SNR_DB but below MIN_CONFIRM_SNR_DB.
        let n = 8192;
        let dc = n / 2;
        let floor = -100.0_f32;
        let peak = floor + 12.0;
        let spectrum = make_spectrum(n, dc, 800, peak, floor);
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 98_000_000);
        // Should give prior candidates but no confirmed (SNR < 15 dB).
        assert!(result.confirmed.is_none());
        assert!(result.candidates.contains(&"FM"));
    }

    #[test]
    fn am_at_aviation_band_confirms_am() {
        let n = 8192;
        let dc = n / 2;
        let spectrum = make_spectrum(n, dc, 40, -20.0, -100.0);
        let iq = am_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 120_000_000);
        assert_eq!(result.confirmed, Some("AM"));
        assert!(result.candidates.contains(&"AM"));
    }

    #[test]
    fn amateur_144_has_multi_candidates() {
        let n = 8192;
        let spectrum = vec![-100.0_f32; n];
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 145_000_000);
        assert!(result.candidates.contains(&"NFM"));
        assert!(result.candidates.contains(&"USB"));
        assert!(result.candidates.contains(&"CW"));
    }

    #[test]
    fn envelope_variance_fm_below_threshold() {
        let var = envelope_variance(&fm_iq(2000));
        assert!(var < 0.15, "FM should have low envelope variance, got {var:.4}");
    }

    #[test]
    fn envelope_variance_am_above_threshold() {
        let var = envelope_variance(&am_iq(2000));
        assert!(var >= 0.15, "AM should have high envelope variance, got {var:.4}");
    }

    #[test]
    fn hz_near_basic() {
        assert!(hz_near(98_000_000, 98_000_000, 1_000));
        assert!(hz_near(98_000_500, 98_000_000, 1_000));
        assert!(!hz_near(98_002_000, 98_000_000, 1_000));
    }

    #[test]
    fn hz_in_basic() {
        assert!(hz_in(98_000_000, 87_500_000, 108_000_000));
        assert!(!hz_in(108_000_000, 87_500_000, 108_000_000));
    }

    // ── New two-tier architecture tests ──────────────────────────────────────

    // Maritime VHF has a single prior ["NFM"] — no spectral analysis should run.
    // Confirmed must be "NFM" even when IQ content doesn't look like NFM.
    #[test]
    fn maritime_vhf_single_prior_confirms_nfm() {
        let n = 8192;
        let dc = n / 2;
        // 50-bin voice-width peak (12.5 kHz), 25 dB SNR.
        let spectrum = make_spectrum(n, dc, 50, -75.0, -100.0);
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 156_800_000);
        assert_eq!(result.confirmed, Some("NFM"));
        assert_eq!(result.candidates, vec!["NFM"]);
    }

    // 2 m amateur has three candidates [NFM, USB, CW]. A symmetric voice-width
    // signal (low asymmetry, not narrow) should resolve to NFM as last resort
    // within the candidate set.
    #[test]
    fn amateur_2m_symmetric_voice_confirms_nfm_from_candidates() {
        let n = 8192;
        let dc = n / 2;
        let spectrum = make_spectrum(n, dc, 50, -75.0, -100.0);
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 145_000_000);
        assert_eq!(result.confirmed, Some("NFM"));
        assert!(result.candidates.contains(&"NFM"));
        assert!(result.candidates.contains(&"USB"));
        assert!(result.candidates.contains(&"CW"));
    }

    // Unknown band (200 MHz), voice-width peak, FM IQ → broad_classify → "NFM".
    #[test]
    fn unknown_band_voice_fm_confirms_nfm() {
        let n = 8192;
        let dc = n / 2;
        let spectrum = make_spectrum(n, dc, 50, -75.0, -100.0);
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 200_000_000);
        assert_eq!(result.confirmed, Some("NFM"));
        assert!(result.candidates.is_empty());
    }

    // ── New band prior tests ──────────────────────────────────────────────────

    #[test]
    fn acars_129mhz_returns_am() {
        let result = classify(&vec![-100.0_f32; 8192], &fm_iq(8192), 2_048_000, 129_125_000);
        assert!(result.candidates.contains(&"AM"), "ACARS prior should return AM");
    }

    #[test]
    fn murs_151mhz_returns_nfm() {
        let result = classify(&vec![-100.0_f32; 8192], &fm_iq(8192), 2_048_000, 152_000_000);
        assert_eq!(result.candidates, vec!["NFM"], "MURS prior should return NFM");
    }

    #[test]
    fn noaa_weather_radio_162mhz_returns_nfm() {
        let result = classify(&vec![-100.0_f32; 8192], &fm_iq(8192), 2_048_000, 162_475_000);
        assert_eq!(result.candidates, vec!["NFM"], "NOAA weather prior should return NFM");
    }

    #[test]
    fn dab3_200mhz_returns_empty_no_audio_mode() {
        let result = classify(&vec![-100.0_f32; 8192], &fm_iq(8192), 2_048_000, 200_000_000);
        assert!(result.candidates.is_empty(), "DAB III prior should have no demodulatable candidates");
    }

    #[test]
    fn frs_gmrs_462mhz_returns_nfm() {
        let result = classify(&vec![-100.0_f32; 8192], &fm_iq(8192), 2_048_000, 462_562_500);
        assert_eq!(result.candidates, vec!["NFM"], "FRS/GMRS prior should return NFM");
    }

    #[test]
    fn public_safety_uhf_460mhz_returns_nfm() {
        let result = classify(&vec![-100.0_f32; 8192], &fm_iq(8192), 2_048_000, 460_000_000);
        assert_eq!(result.candidates, vec!["NFM"], "Public safety UHF prior should return NFM");
    }

    // Unknown band, narrow peak (<3 kHz) → broad_classify returns None — no guess
    // without a prior (avoids cycling between CW / data tone / noise artefact).
    #[test]
    fn unknown_band_narrow_signal_gives_no_confirmed() {
        let n = 8192;
        let dc = n / 2;
        // 8 bins × 250 Hz/bin = 2 kHz — BwFamily::Narrow
        let spectrum = make_spectrum(n, dc, 8, -75.0, -100.0);
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 200_000_000);
        assert!(result.confirmed.is_none());
        assert!(result.candidates.is_empty());
    }

    // Unknown band, wideband FM signal → broad_classify → "FM".
    #[test]
    fn unknown_band_wideband_signal_confirms_fm() {
        let n = 8192;
        let dc = n / 2;
        // 800 bins × 250 Hz/bin = 200 kHz — BwFamily::Wideband
        let spectrum = make_spectrum(n, dc, 800, -20.0, -100.0);
        let iq = fm_iq(n);
        let result = classify(&spectrum, &iq, 2_048_000, 200_000_000);
        assert_eq!(result.confirmed, Some("FM"));
        assert!(result.candidates.is_empty());
    }

    // Aviation band has single prior ["AM"]. Even with FM IQ (wrong type), the
    // single-prior path must win — proves spectral analysis is skipped entirely.
    #[test]
    fn aviation_band_single_prior_always_confirms_am() {
        let n = 8192;
        let dc = n / 2;
        let spectrum = make_spectrum(n, dc, 40, -20.0, -100.0);
        let iq = fm_iq(n); // deliberately wrong IQ type
        let result = classify(&spectrum, &iq, 2_048_000, 125_000_000);
        assert_eq!(result.confirmed, Some("AM"));
        assert_eq!(result.candidates, vec!["AM"]);
    }
}
