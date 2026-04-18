//! PCM float32 WAV writer (mono).
//!
//! Used by the audio-recording path to persist demodulated samples at
//! `AUDIO_RATE_HZ` (44.1 kHz) to disk. See `docs/SIGNALS.md` §2: audio
//! recordings are WAV, not SigMF (SigMF is for raw IQ only).
//!
//! Two flavours:
//! - [`write_mono_f32`]: one-shot writer, used by tests.
//! - [`WavStreamWriter`]: streaming writer with a placeholder header
//!   that gets patched on `finalize`. The DSP task appends chunks
//!   whenever the demod emits them; stop may happen seconds or hours
//!   later.
//!
//! Web Audio's `decodeAudioData` consumes this layout natively.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::RailError;

/// `WAVE_FORMAT_IEEE_FLOAT` (0x0003) — non-PCM float samples.
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const BITS_PER_SAMPLE: u16 = 32;
const BYTES_PER_SAMPLE: u32 = 4;
const FMT_CHUNK_SIZE: u32 = 16;

/// Write `samples` (mono, f32) to `path` as a WAV file at `sample_rate_hz`.
///
/// The file is written in full before `rename`-ing into place so a
/// crash mid-write cannot leave a truncated artifact.
pub fn write_mono_f32<P: AsRef<Path>>(
    path: P,
    samples: &[f32],
    sample_rate_hz: u32,
) -> Result<(), RailError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RailError::CaptureError(format!("wav dir: {e}")))?;
    }
    let tmp = path.with_extension("wav.tmp");
    {
        let file = File::create(&tmp)
            .map_err(|e| RailError::CaptureError(format!("wav create: {e}")))?;
        let mut w = BufWriter::new(file);
        let header = build_header(samples.len() as u32, sample_rate_hz);
        w.write_all(&header)
            .map_err(|e| RailError::CaptureError(format!("wav header: {e}")))?;
        for &s in samples {
            w.write_all(&s.to_le_bytes())
                .map_err(|e| RailError::CaptureError(format!("wav sample: {e}")))?;
        }
        w.flush()
            .map_err(|e| RailError::CaptureError(format!("wav flush: {e}")))?;
        w.get_ref()
            .sync_all()
            .map_err(|e| RailError::CaptureError(format!("wav sync: {e}")))?;
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| RailError::CaptureError(format!("wav rename: {e}")))
}

fn build_header(sample_count: u32, sample_rate_hz: u32) -> [u8; 44] {
    let channels: u16 = 1;
    let block_align = channels as u32 * BYTES_PER_SAMPLE;
    let byte_rate = sample_rate_hz * block_align;
    let data_size = sample_count * BYTES_PER_SAMPLE;
    let riff_size = 36 + data_size;

    let mut buf = [0u8; 44];
    buf[0..4].copy_from_slice(b"RIFF");
    buf[4..8].copy_from_slice(&riff_size.to_le_bytes());
    buf[8..12].copy_from_slice(b"WAVE");
    buf[12..16].copy_from_slice(b"fmt ");
    buf[16..20].copy_from_slice(&FMT_CHUNK_SIZE.to_le_bytes());
    buf[20..22].copy_from_slice(&WAVE_FORMAT_IEEE_FLOAT.to_le_bytes());
    buf[22..24].copy_from_slice(&channels.to_le_bytes());
    buf[24..28].copy_from_slice(&sample_rate_hz.to_le_bytes());
    buf[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    buf[32..34].copy_from_slice(&(block_align as u16).to_le_bytes());
    buf[34..36].copy_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    buf[36..40].copy_from_slice(b"data");
    buf[40..44].copy_from_slice(&data_size.to_le_bytes());
    buf
}

/// Streaming WAV writer. `append` is cheap (buffered writes); the size
/// fields in the header are only correct after [`WavStreamWriter::finalize`].
pub struct WavStreamWriter {
    file: BufWriter<File>,
    path: PathBuf,
    sample_rate_hz: u32,
    samples_written: u64,
}

impl WavStreamWriter {
    /// Create `path`, write a placeholder header (zero samples), and
    /// position the cursor for appending data.
    pub fn create(path: &Path, sample_rate_hz: u32) -> Result<Self, RailError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RailError::CaptureError(format!("wav dir: {e}")))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|e| RailError::CaptureError(format!("wav create: {e}")))?;
        let mut w = BufWriter::new(file);
        w.write_all(&build_header(0, sample_rate_hz))
            .map_err(|e| RailError::CaptureError(format!("wav header: {e}")))?;
        Ok(Self {
            file: w,
            path: path.to_path_buf(),
            sample_rate_hz,
            samples_written: 0,
        })
    }

    /// Append `samples` as little-endian f32 bytes.
    pub fn append(&mut self, samples: &[f32]) -> Result<(), RailError> {
        if samples.is_empty() {
            return Ok(());
        }
        // One copy into a scratch byte buffer, then a single write:
        // amortises the per-sample write_all cost at 44 kHz.
        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for &s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.file
            .write_all(&bytes)
            .map_err(|e| RailError::CaptureError(format!("wav sample: {e}")))?;
        self.samples_written += samples.len() as u64;
        Ok(())
    }

    /// Finalize the file: flush the buffer, seek back to the two size
    /// fields and patch them with the actual counts, then `sync_all`.
    /// Returns the total sample count written.
    pub fn finalize(mut self) -> Result<u64, RailError> {
        self.file
            .flush()
            .map_err(|e| RailError::CaptureError(format!("wav flush: {e}")))?;
        // Clamp to u32 for the RIFF size fields. The spec can't express
        // anything larger; if someone really records >24 hours of mono
        // f32 at 44.1 kHz, the file will still be playable but the
        // size field will wrap — that's a WAV-spec limitation, not ours.
        let sample_count = u32::try_from(self.samples_written).unwrap_or(u32::MAX);
        let data_size = sample_count.saturating_mul(BYTES_PER_SAMPLE);
        let riff_size = 36u32.saturating_add(data_size);

        let file = self.file.get_mut();
        file.seek(SeekFrom::Start(4))
            .map_err(|e| RailError::CaptureError(format!("wav seek riff: {e}")))?;
        file.write_all(&riff_size.to_le_bytes())
            .map_err(|e| RailError::CaptureError(format!("wav patch riff: {e}")))?;
        file.seek(SeekFrom::Start(40))
            .map_err(|e| RailError::CaptureError(format!("wav seek data: {e}")))?;
        file.write_all(&data_size.to_le_bytes())
            .map_err(|e| RailError::CaptureError(format!("wav patch data: {e}")))?;
        file.sync_all()
            .map_err(|e| RailError::CaptureError(format!("wav sync: {e}")))?;
        Ok(self.samples_written)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
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
            "rail-wav-test-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p
    }

    #[test]
    fn header_fields_match_spec() {
        let path = tmp("header");
        let samples = [0.0_f32, 0.5, -0.5, 1.0];
        write_mono_f32(&path, &samples, 44_100).unwrap();

        let mut file = File::open(&path).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            44_100
        );
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 32);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            (samples.len() as u32) * 4
        );
        assert_eq!(bytes.len(), 44 + samples.len() * 4);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn stream_writer_patches_header() {
        let path = tmp("stream");
        let mut w = WavStreamWriter::create(&path, 44_100).unwrap();
        w.append(&[0.1, 0.2, 0.3]).unwrap();
        w.append(&[-0.4, 0.5]).unwrap();
        let count = w.finalize().unwrap();
        assert_eq!(count, 5);

        let mut bytes = Vec::new();
        File::open(&path)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes.len(), 44 + 5 * 4);
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            5 * 4
        );
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            36 + 5 * 4
        );
        let payload = &bytes[44..];
        for (i, chunk) in payload.chunks_exact(4).enumerate() {
            let got = f32::from_le_bytes(chunk.try_into().unwrap());
            let expected = [0.1_f32, 0.2, 0.3, -0.4, 0.5][i];
            assert_eq!(got.to_bits(), expected.to_bits());
        }
        std::fs::remove_file(&path).ok();
    }
}
