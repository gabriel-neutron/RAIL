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
    #[serde(rename = "core:datatype")]
    pub datatype: String,
    #[serde(rename = "core:sample_rate")]
    pub sample_rate: u64,
    #[serde(rename = "core:version")]
    pub version: String,
    #[serde(rename = "core:description")]
    pub description: String,
    #[serde(rename = "core:author")]
    pub author: String,
    #[serde(rename = "rail:center_frequency_hz")]
    pub center_frequency_hz: u64,
    #[serde(rename = "rail:tuner_gain_db")]
    pub tuner_gain_db: f32,
    #[serde(rename = "rail:demod_mode")]
    pub demod_mode: String,
    #[serde(rename = "rail:filter_bandwidth_hz")]
    pub filter_bandwidth_hz: u32,
}

/// Per-capture entry inside the `captures` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigMfCapture {
    #[serde(rename = "core:sample_start")]
    pub sample_start: u64,
    #[serde(rename = "core:datetime")]
    pub datetime: String,
    #[serde(rename = "core:frequency")]
    pub frequency: u64,
}

/// Full sigmf-meta document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigMfMeta {
    pub global: SigMfGlobal,
    pub captures: Vec<SigMfCapture>,
    pub annotations: Vec<Value>,
}

/// Parameters the caller pins at record-start time. They end up
/// verbatim in the finalized `.sigmf-meta`.
#[derive(Debug, Clone)]
pub struct SigMfStartParams {
    pub sample_rate_hz: u32,
    pub center_frequency_hz: u64,
    pub tuner_gain_db: f32,
    pub demod_mode: String,
    pub filter_bandwidth_hz: u32,
    pub datetime_iso8601: String,
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

    pub fn data_path(&self) -> &Path {
        &self.data_path
    }

    pub fn meta_path(&self) -> &Path {
        &self.meta_path
    }

    pub fn samples_written(&self) -> u64 {
        self.samples_written
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp(label: &str) -> std::path::PathBuf {
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
            datetime_iso8601: "2024-01-01T12:00:00Z".into(),
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
        for (i, pair) in data.chunks_exact(8).enumerate() {
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
}
