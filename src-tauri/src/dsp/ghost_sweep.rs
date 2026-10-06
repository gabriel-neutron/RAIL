//! End-to-end centre sweep over a SYNTHETIC carrier (issue #24).
//!
//! Every sample in this file is synthetic: a single tone at a known absolute
//! frequency, generated here, never captured from a dongle. The SigMF files it
//! writes say so in `core:description`. The sweep walks the tuner centre
//! across the tone through the same arithmetic the live tune path uses
//! (`LO = centre − fs/4`, label = `read-back LO + fs/4`), runs the live DSP
//! chain (`iq_u8_to_complex` → `apply_fs4_shift` → FFT) and reads the peak back
//! with the display formula of `docs/DSP.md` §9.5. The frontend half of the
//! chain is covered by `src/viewport/apparentFrequency.test.ts`.
//!
//! Troubleshooting table: `docs/DSP.md` §8 "Ghost signals".

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use num_complex::Complex;

use crate::capture::sigmf::{SigMfMeta, SigMfStartParams, SigMfStreamWriter};
use crate::dsp::fft::FftProcessor;
use crate::dsp::waterfall::{apply_fs4_shift, iq_u8_to_complex};
use crate::hardware::fake_tuner::FakeTuner;
use crate::hardware::tuner::Tuner;
use crate::ipc::session::live::lo_offset_hz;
use crate::replay::load_info;

const SAMPLE_RATE_HZ: u32 = 2_048_000;
const FFT_SIZE: usize = 8192;
/// Absolute frequency of the synthetic carrier, in Hz.
const SYNTHETIC_CARRIER_HZ: f64 = 100_300_000.0;
/// Tone amplitude as a fraction of full scale.
const TONE_AMPLITUDE: f64 = 0.8;
/// Tuner frequency grid the fake read-back snaps down to, in Hz.
const TUNER_SNAP_HZ: u32 = 1_000;
/// One FFT bin, in Hz.
const ONE_BIN_HZ: f64 = SAMPLE_RATE_HZ as f64 / FFT_SIZE as f64;
/// Centre offsets from the carrier swept by the tests, in Hz.
const SWEEP_OFFSETS_HZ: [i64; 17] = [
    -800_000, -600_000, -400_000, -250_000, -100_000, -50_000, -10_000, -300, 0, 300, 10_000,
    50_000, 100_000, 250_000, 400_000, 600_000, 800_000,
];

/// How the simulated chain is deliberately broken, to prove the sweep can go red.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    /// The chain as shipped.
    None,
    /// Hypothesis H2: IQ conjugated, the spectrum mirrored about the centre.
    Mirror,
    /// Hypothesis H3: the fs/4 mixer applied with the wrong sign.
    WrongShiftSign,
}

/// The `n` u8 IQ samples the dongle would emit for the synthetic carrier with
/// its LO parked at `lo_hz`. Returns `None` when the carrier would alias.
fn analog_u8_iq(lo_hz: u32, n: usize) -> Option<Vec<u8>> {
    let baseband_hz = SYNTHETIC_CARRIER_HZ - f64::from(lo_hz);
    if baseband_hz.abs() >= f64::from(SAMPLE_RATE_HZ) / 2.0 {
        return None;
    }
    let to_byte =
        |v: f64| (((v * TONE_AMPLITUDE + 1.0) * 127.5).round() as i32).clamp(0, 255) as u8;
    let mut raw = Vec::with_capacity(2 * n);
    for k in 0..n {
        let phase = std::f64::consts::TAU * baseband_hz * k as f64 / f64::from(SAMPLE_RATE_HZ);
        raw.push(to_byte(phase.cos()));
        raw.push(to_byte(phase.sin()));
    }
    Some(raw)
}

/// Run the live conversion and mixer on `raw`, with `fault` applied.
fn shifted_iq(raw: &[u8], fault: Fault) -> Vec<Complex<f32>> {
    let mut iq = vec![Complex::new(0.0_f32, 0.0); raw.len() / 2];
    iq_u8_to_complex(raw, &mut iq).unwrap();
    let conj_all = |iq: &mut [Complex<f32>]| iq.iter_mut().for_each(|s| *s = s.conj());
    match fault {
        Fault::None => {
            apply_fs4_shift(&mut iq, 0);
        }
        Fault::Mirror => {
            apply_fs4_shift(&mut iq, 0);
            conj_all(&mut iq);
        }
        Fault::WrongShiftSign => {
            // conj, shift, conj is exp(+jπn/2) through the shipped mixer.
            conj_all(&mut iq);
            apply_fs4_shift(&mut iq, 0);
            conj_all(&mut iq);
        }
    }
    iq
}

/// Apparent frequency of the strongest bin, by the `docs/DSP.md` §9.5 display
/// formula: the tuned label sits at the centre of bin `N/2`.
fn apparent_hz(iq: &[Complex<f32>], label_hz: f64) -> f64 {
    let spectrum = FftProcessor::new(FFT_SIZE).process(iq).to_vec();
    let peak = spectrum
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap();
    label_hz + (peak as f64 - FFT_SIZE as f64 / 2.0) * ONE_BIN_HZ
}

/// Tune to `centre_hz` the way `commands::set_frequency` does and return the
/// LO the tuner reports plus the label the DSP task is told.
fn tune(tuner: &FakeTuner, centre_hz: u32) -> (u32, f64) {
    let offset = lo_offset_hz(SAMPLE_RATE_HZ);
    tuner.set_center_freq(centre_hz - offset).unwrap();
    let lo_hz = tuner.center_freq();
    (lo_hz, f64::from(lo_hz + offset))
}

/// Round-trip one centre through a SYNTHETIC SigMF capture and back.
fn through_sigmf(shifted: &[Complex<f32>], label_hz: f64, seq: usize) -> (Vec<Complex<f32>>, f64) {
    let dir = std::env::temp_dir().join(format!("rail-ghost-{}-{seq}", std::process::id()));
    let data = dir.join("synthetic.sigmf-data");
    let meta = dir.join("synthetic.sigmf-meta");
    let mut w = SigMfStreamWriter::create(
        &meta,
        &data,
        SigMfStartParams {
            sample_rate_hz: SAMPLE_RATE_HZ,
            center_frequency_hz: label_hz as u64,
            tuner_gain_db: 0.0,
            demod_mode: "FM".into(),
            filter_bandwidth_hz: 200_000,
            squelch_dbfs: None,
            datetime_iso8601: "1970-01-01T00:00:00Z".into(),
            signal_type_guess: None,
        },
    )
    .unwrap();
    w.append_shifted(shifted).unwrap();
    w.finalize().unwrap();

    let mut doc: SigMfMeta = serde_json::from_slice(&std::fs::read(&meta).unwrap()).unwrap();
    doc.global.description =
        "SYNTHETIC TEST DATA: one tone at a known absolute frequency (issue #24)".into();
    std::fs::write(&meta, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();

    let info = load_info(&data).unwrap();
    let bytes = std::fs::read(&data).unwrap();
    let samples = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| {
            let re = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            let im = f32::from_le_bytes([c[4], c[5], c[6], c[7]]);
            Complex::new(re, im)
        })
        .collect();
    (samples, info.center_frequency_hz as f64)
}

/// `(offset, apparent − true)` at every swept centre, for the chain with `fault`.
fn sweep_errors_hz(fault: Fault, via_sigmf: bool) -> Vec<(i64, f64)> {
    let tuner = FakeTuner::new(TUNER_SNAP_HZ);
    let mut out = Vec::new();
    for (seq, &offset) in SWEEP_OFFSETS_HZ.iter().enumerate() {
        let centre_hz = (SYNTHETIC_CARRIER_HZ as i64 + offset) as u32;
        let (lo_hz, label_hz) = tune(&tuner, centre_hz);
        let Some(raw) = analog_u8_iq(lo_hz, FFT_SIZE) else {
            continue;
        };
        let shifted = shifted_iq(&raw, fault);
        let (iq, label_hz) = if via_sigmf {
            through_sigmf(&shifted, label_hz, seq)
        } else {
            (shifted, label_hz)
        };
        out.push((offset, apparent_hz(&iq, label_hz) - SYNTHETIC_CARRIER_HZ));
    }
    out
}

#[test]
fn apparent_equals_true_within_one_bin_at_every_centre() {
    for via_sigmf in [false, true] {
        let errors = sweep_errors_hz(Fault::None, via_sigmf);
        assert!(
            errors.len() >= 15,
            "sweep skipped too many centres: {errors:?}"
        );
        for (offset, err) in errors {
            assert!(
                err.abs() <= ONE_BIN_HZ,
                "centre {offset:+} Hz from the carrier (sigmf={via_sigmf}): apparent - true = {err} Hz"
            );
        }
    }
}

#[test]
fn a_mirrored_chain_is_caught_with_the_h2_signature() {
    // H2: the apparent offset from the centre is the negative of the true one,
    // so apparent − true = 2 · (centre − carrier) = 2 · offset.
    for (offset, err) in sweep_errors_hz(Fault::Mirror, false) {
        if offset.abs() < 3 * ONE_BIN_HZ as i64 {
            continue;
        }
        assert!(
            (err - 2.0 * offset as f64).abs() <= 2.0 * ONE_BIN_HZ,
            "{offset}: {err}"
        );
    }
}

#[test]
fn a_wrong_sign_mixer_is_caught_with_the_h3_signature() {
    // H3: the carrier lands about fs/2 away from where the label says.
    for (offset, err) in sweep_errors_hz(Fault::WrongShiftSign, false) {
        assert!(err.abs() > 100.0 * ONE_BIN_HZ, "{offset}: {err}");
    }
}
