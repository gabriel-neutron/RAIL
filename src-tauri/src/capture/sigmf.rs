//! SigMF (`.sigmf-meta` + `.sigmf-data`) streaming writer.
//!
//! See `docs/SIGNALS.md` §1. RAIL stores IQ as `cf32_le` — raw
//! little-endian interleaved float32 complex samples. The writer
//! consumes samples that are already normalized and `fs/4`-shifted
//! (which the DSP task computes once per iteration for the waterfall
//! and demod anyway), so long captures stay phase-continuous and the
//! writer does no DSP work itself.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use num_complex::Complex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::RailError;

/// Global SigMF metadata block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigMfGlobal {
    /// SigMF sample datatype string; RAIL always writes `cf32_le`.
    #[serde(rename = "core:datatype")]
    pub datatype: String,
    /// Sample rate of the recorded IQ stream, in Hz.
    #[serde(rename = "core:sample_rate")]
    pub sample_rate: u64,
    /// SigMF specification version the file conforms to.
    #[serde(rename = "core:version")]
    pub version: String,
    /// Free-text description of the capture.
    #[serde(rename = "core:description")]
    pub description: String,
    /// Author string written into the metadata.
    #[serde(rename = "core:author")]
    pub author: String,
    /// Tuner centre frequency at capture-start time, in Hz.
    #[serde(rename = "rail:center_frequency_hz")]
    pub center_frequency_hz: u64,
    /// Tuner gain at capture-start time, in dB.
    #[serde(rename = "rail:tuner_gain_db")]
    pub tuner_gain_db: f32,
    /// Demodulator mode active at capture-start time, as its wire-name.
    #[serde(rename = "rail:demod_mode")]
    pub demod_mode: String,
    /// Channel filter bandwidth at capture-start time, in Hz.
    #[serde(rename = "rail:filter_bandwidth_hz")]
    pub filter_bandwidth_hz: u32,
    /// Classifier label at the time of capture, e.g. `"WBFM"`. `None`
    /// when no stream was running or the classifier produced no result.
    /// See `docs/SIGNALS.md` §5.4.
    #[serde(
        rename = "rail:signal_type_guess",
        skip_serializing_if = "Option::is_none"
    )]
    pub signal_type_guess: Option<String>,
    /// Squelch threshold in dBFS at capture-start time. `None` when the
    /// gate was disabled. Optional and defaulted so captures written
    /// before this field existed still decode strictly (see
    /// `crate::replay::load_info`).
    #[serde(
        rename = "rail:squelch_dbfs",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub squelch_dbfs: Option<f32>,
}

/// Per-capture entry inside the `captures` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigMfCapture {
    /// Index of the first sample this entry describes, in samples from the start of the data file.
    #[serde(rename = "core:sample_start")]
    pub sample_start: u64,
    /// ISO 8601 timestamp of the first sample.
    #[serde(rename = "core:datetime")]
    pub datetime: String,
    /// Centre frequency for this capture segment, in Hz.
    #[serde(rename = "core:frequency")]
    pub frequency: u64,
}

/// Full sigmf-meta document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigMfMeta {
    /// Global metadata block.
    pub global: SigMfGlobal,
    /// Capture segments, in ascending `sample_start` order.
    pub captures: Vec<SigMfCapture>,
    /// Annotation objects; RAIL writes none today and preserves what it reads.
    pub annotations: Vec<Value>,
}

/// Parameters the caller pins at record-start time. They end up
/// verbatim in the finalized `.sigmf-meta`.
#[derive(Debug, Clone)]
pub struct SigMfStartParams {
    /// Sample rate of the IQ stream to record, in Hz.
    pub sample_rate_hz: u32,
    /// Tuner centre frequency, in Hz.
    pub center_frequency_hz: u64,
    /// Tuner gain, in dB.
    pub tuner_gain_db: f32,
    /// Demodulator mode wire-name.
    pub demod_mode: String,
    /// Channel filter bandwidth, in Hz.
    pub filter_bandwidth_hz: u32,
    /// Squelch threshold in dBFS; `None` when the gate is disabled.
    pub squelch_dbfs: Option<f32>,
    /// ISO 8601 timestamp of the first recorded sample.
    pub datetime_iso8601: String,
    /// Classifier label at capture-start time. Forwarded verbatim into
    /// `rail:signal_type_guess` in the finalized `.sigmf-meta`.
    pub signal_type_guess: Option<String>,
}

/// Streaming writer: appends `cf32_le` bytes to `<data_path>` as samples
/// arrive, then writes the companion `.sigmf-meta` JSON on `finalize`.
pub struct SigMfStreamWriter {
    data: BufWriter<File>,
    data_path: PathBuf,
    meta_path: PathBuf,
    params: SigMfStartParams,
    samples_written: u64,
}

impl SigMfStreamWriter {
    /// Create `data_path`, write nothing to `meta_path` yet (it's
    /// produced on `finalize`). `data_path` is truncated if it exists.
    pub fn create(
        meta_path: &Path,
        data_path: &Path,
        params: SigMfStartParams,
    ) -> Result<Self, RailError> {
        if let Some(parent) = data_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RailError::CaptureError(format!("sigmf dir: {e}")))?;
        }
        let file = File::create(data_path)
            .map_err(|e| RailError::CaptureError(format!("sigmf data create: {e}")))?;
        Ok(Self {
            data: BufWriter::with_capacity(256 * 1024, file),
            data_path: data_path.to_path_buf(),
            meta_path: meta_path.to_path_buf(),
            params,
            samples_written: 0,
        })
    }

    /// Append already-shifted, already-normalized complex samples as
    /// interleaved `cf32_le` bytes.
    pub fn append_shifted(&mut self, samples: &[Complex<f32>]) -> Result<(), RailError> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(samples.len() * 8);
        for s in samples {
            bytes.extend_from_slice(&s.re.to_le_bytes());
            bytes.extend_from_slice(&s.im.to_le_bytes());
        }
        self.data
            .write_all(&bytes)
            .map_err(|e| RailError::CaptureError(format!("sigmf data write: {e}")))?;
        self.samples_written += samples.len() as u64;
        Ok(())
    }

    /// Close the data file and write the accompanying `.sigmf-meta`
    /// JSON. Returns the final sample count.
    pub fn finalize(mut self) -> Result<u64, RailError> {
        self.data
            .flush()
            .map_err(|e| RailError::CaptureError(format!("sigmf data flush: {e}")))?;
        self.data
            .get_ref()
            .sync_all()
            .map_err(|e| RailError::CaptureError(format!("sigmf data sync: {e}")))?;
        drop(self.data);

        let meta = SigMfMeta {
            global: SigMfGlobal {
                datatype: "cf32_le".into(),
                sample_rate: self.params.sample_rate_hz as u64,
                version: "1.0.0".into(),
                description: "RAIL IQ capture".into(),
                author: "RAIL".into(),
                center_frequency_hz: self.params.center_frequency_hz,
                tuner_gain_db: self.params.tuner_gain_db,
                demod_mode: self.params.demod_mode,
                filter_bandwidth_hz: self.params.filter_bandwidth_hz,
                signal_type_guess: self.params.signal_type_guess.clone(),
                squelch_dbfs: self.params.squelch_dbfs,
            },
            captures: vec![SigMfCapture {
                sample_start: 0,
                datetime: self.params.datetime_iso8601,
                frequency: self.params.center_frequency_hz,
            }],
            annotations: Vec::new(),
        };
        let body = serde_json::to_vec_pretty(&meta)
            .map_err(|e| RailError::CaptureError(format!("sigmf meta serialize: {e}")))?;
        let tmp = self.meta_path.with_extension("sigmf-meta.tmp");
        std::fs::write(&tmp, &body)
            .map_err(|e| RailError::CaptureError(format!("sigmf meta write: {e}")))?;
        std::fs::rename(&tmp, &self.meta_path)
            .map_err(|e| RailError::CaptureError(format!("sigmf meta rename: {e}")))?;
        Ok(self.samples_written)
    }

    /// Path of the `.sigmf-data` file being written.
    pub fn data_path(&self) -> &Path {
        &self.data_path
    }

    /// Path of the `.sigmf-meta` file finalized on close.
    pub fn meta_path(&self) -> &Path {
        &self.meta_path
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::io::Read;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp(label: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "rail-sigmf-test-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn fixture_params() -> SigMfStartParams {
        SigMfStartParams {
            sample_rate_hz: 2_048_000,
            center_frequency_hz: 100_000_000,
            tuner_gain_db: 30.0,
            demod_mode: "FM".into(),
            filter_bandwidth_hz: 200_000,
            squelch_dbfs: Some(-55.0),
            datetime_iso8601: "2024-01-01T12:00:00Z".into(),
            signal_type_guess: Some("WBFM".into()),
        }
    }

    #[test]
    fn stream_writer_roundtrips_and_emits_meta() {
        let dir = tmp("stream");
        let meta_path = dir.join("clip.sigmf-meta");
        let data_path = dir.join("clip.sigmf-data");

        let mut w = SigMfStreamWriter::create(&meta_path, &data_path, fixture_params()).unwrap();
        let first: Vec<Complex<f32>> = (0..4)
            .map(|k| Complex::new(k as f32 * 0.1, -(k as f32) * 0.1))
            .collect();
        let second: Vec<Complex<f32>> = (4..7)
            .map(|k| Complex::new(k as f32 * 0.1, -(k as f32) * 0.1))
            .collect();
        w.append_shifted(&first).unwrap();
        w.append_shifted(&second).unwrap();
        let count = w.finalize().unwrap();
        assert_eq!(count, 7);

        // Data file contains exactly 7 * 8 = 56 bytes.
        let mut data = Vec::new();
        File::open(&data_path)
            .unwrap()
            .read_to_end(&mut data)
            .unwrap();
        assert_eq!(data.len(), 7 * 8);
        for (i, pair) in data.as_chunks::<8>().0.iter().enumerate() {
            let re = f32::from_le_bytes(pair[0..4].try_into().unwrap());
            let im = f32::from_le_bytes(pair[4..8].try_into().unwrap());
            assert!((re - (i as f32 * 0.1)).abs() < 1e-6);
            assert!((im - -(i as f32 * 0.1)).abs() < 1e-6);
        }

        // Meta file parses and carries the `rail:` namespace fields.
        let meta_text = std::fs::read_to_string(&meta_path).unwrap();
        let v: Value = serde_json::from_str(&meta_text).unwrap();
        assert_eq!(v["global"]["core:datatype"], "cf32_le");
        assert_eq!(v["global"]["core:sample_rate"], 2_048_000);
        assert_eq!(v["global"]["rail:center_frequency_hz"], 100_000_000);
        assert_eq!(v["global"]["rail:demod_mode"], "FM");
        assert_eq!(v["captures"][0]["core:datetime"], "2024-01-01T12:00:00Z");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn squelch_round_trips_through_meta() {
        let dir = tmp("squelch");
        let meta_path = dir.join("clip.sigmf-meta");
        let data_path = dir.join("clip.sigmf-data");

        let w = SigMfStreamWriter::create(&meta_path, &data_path, fixture_params()).unwrap();
        w.finalize().unwrap();

        let text = std::fs::read_to_string(&meta_path).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["global"]["rail:squelch_dbfs"], -55.0);
        let meta: SigMfMeta = serde_json::from_str(&text).unwrap();
        assert_eq!(meta.global.squelch_dbfs, Some(-55.0));

        // Gate disabled: the field is omitted rather than written as a
        // non-finite float, which serde_json cannot read back.
        let mut off = fixture_params();
        off.squelch_dbfs = None;
        let w = SigMfStreamWriter::create(&meta_path, &data_path, off).unwrap();
        w.finalize().unwrap();
        let text = std::fs::read_to_string(&meta_path).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(v["global"].get("rail:squelch_dbfs").is_none());
        let meta: SigMfMeta = serde_json::from_str(&text).unwrap();
        assert_eq!(meta.global.squelch_dbfs, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn meta_without_squelch_field_still_decodes() {
        // Every capture written before this field existed — including
        // docs/assets/demo_iq.sigmf-meta — must still take the strict
        // `from_value::<SigMfMeta>` path in `crate::replay::load_info`,
        // not the loose fallback.
        let raw = serde_json::json!({
            "global": {
                "core:datatype": "cf32_le",
                "core:sample_rate": 2_048_000,
                "core:version": "1.0.0",
                "core:description": "RAIL IQ capture",
                "core:author": "RAIL",
                "rail:center_frequency_hz": 100_000_000,
                "rail:tuner_gain_db": 30.0,
                "rail:demod_mode": "FM",
                "rail:filter_bandwidth_hz": 200_000
            },
            "captures": [{
                "core:sample_start": 0,
                "core:datetime": "2024-01-01T12:00:00Z",
                "core:frequency": 100_000_000
            }],
            "annotations": []
        });
        let meta: SigMfMeta = serde_json::from_value(raw).unwrap();
        assert_eq!(meta.global.squelch_dbfs, None);
        assert_eq!(meta.global.center_frequency_hz, 100_000_000);
    }

    #[test]
    fn missing_squelch_is_not_what_blocks_the_shipped_demo_capture() {
        // The shipped demo already fails the strict decode, but for an
        // unrelated pre-existing reason: `rail:tuner_gain_db` is a bare f32
        // and AGC captures write it as `null`. Patch only that field and the
        // strict path accepts the file with no `rail:squelch_dbfs` present —
        // so the new field costs no back-compatibility.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("docs")
            .join("assets")
            .join("demo_iq.sigmf-meta");
        if !path.exists() {
            return;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut raw: Value = serde_json::from_str(&text).unwrap();
        assert!(raw["global"].get("rail:squelch_dbfs").is_none());
        assert!(raw["global"]["rail:tuner_gain_db"].is_null());

        raw["global"]["rail:tuner_gain_db"] = serde_json::json!(30.0);
        let meta: SigMfMeta = serde_json::from_value(raw).unwrap();
        assert_eq!(meta.global.squelch_dbfs, None);
        assert_eq!(meta.global.center_frequency_hz, 101_583_820);
    }
}
