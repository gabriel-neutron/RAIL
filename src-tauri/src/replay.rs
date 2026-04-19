//! SigMF IQ-file replay.
//!
//! Reads a `.sigmf-data` file at real-time pace and feeds the samples
//! into the existing DSP task as [`DspInput::Cf32Shifted`]. The
//! accompanying `.sigmf-meta` is parsed once to seed the session with
//! the capture's sample rate, centre frequency and demod hints so the
//! UI reflects the file and not the live radio.
//!
//! Transport (play / pause / seek / stop) is driven by a
//! [`ReplayControl`] channel the Tauri commands write into. Position
//! updates fan out to the frontend via the `replay-position` event
//! (see [`ReplayPosition`]).

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use num_complex::Complex;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::capture::sigmf::SigMfMeta;
use crate::dsp::input::DspInput;
use crate::error::RailError;
use crate::ipc::dsp_task::FFT_SIZE;
use crate::ipc::events::ReplayPosition;

/// Bytes per cf32 sample on disk (I f32 + Q f32, little-endian).
const BYTES_PER_SAMPLE: u64 = 8;

/// Target chunk length in real-time ms. 40 ms matches the waterfall
/// emit cadence (~25 fps, `docs/DSP.md` §3) so the spectrum paint
/// loop stays smooth.
const CHUNK_MS: u64 = 40;

/// Interval between `replay-position` events.
const POSITION_EMIT_INTERVAL: Duration = Duration::from_millis(40);

/// Number of waterfall rows to backfill on seek. Matches
/// `WATERFALL_HEIGHT` in `src/components/Waterfall/index.tsx` — the
/// full canvas height so a seek fills the view in one shot. With
/// `CHUNK_MS = 40`, this is ~14.4 s of file-time history.
const WATERFALL_PREFILL_ROWS: usize = 360;

/// Parsed metadata for an on-disk IQ capture.
#[derive(Debug, Clone)]
pub struct ReplayInfo {
    pub data_path: PathBuf,
    pub meta_path: PathBuf,
    pub sample_rate_hz: u32,
    pub center_frequency_hz: u64,
    pub demod_mode: String,
    pub filter_bandwidth_hz: u32,
    pub total_samples: u64,
    pub datetime_iso8601: String,
}

impl ReplayInfo {
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate_hz == 0 {
            return 0;
        }
        self.total_samples.saturating_mul(1_000) / self.sample_rate_hz as u64
    }
}

/// Transport commands the Tauri layer sends to the reader task.
pub enum ReplayControl {
    Play,
    Pause,
    /// Absolute sample index. The reader clamps to `total_samples`.
    Seek {
        sample_idx: u64,
    },
    Stop,
}

/// Derive the sibling `.sigmf-meta` path from a `.sigmf-data` path by
/// swapping the extension. Falls back to appending `.sigmf-meta`.
pub fn meta_path_for(data_path: &Path) -> PathBuf {
    let lossy = data_path.to_string_lossy();
    if let Some(stripped) = lossy.strip_suffix(".sigmf-data") {
        PathBuf::from(format!("{stripped}.sigmf-meta"))
    } else {
        let mut p = data_path.to_path_buf();
        p.set_extension("sigmf-meta");
        p
    }
}

/// Load metadata and derive sample count from the data file size.
pub fn load_info(data_path: &Path) -> Result<ReplayInfo, RailError> {
    let meta_path = meta_path_for(data_path);
    if !data_path.exists() {
        return Err(RailError::CaptureError(format!(
            "IQ data file not found: {}",
            data_path.display()
        )));
    }
    if !meta_path.exists() {
        return Err(RailError::CaptureError(format!(
            "SigMF meta file not found: {}",
            meta_path.display()
        )));
    }

    let meta_bytes = std::fs::read(&meta_path)
        .map_err(|e| RailError::CaptureError(format!("sigmf meta read: {e}")))?;
    let raw: Value = serde_json::from_slice(&meta_bytes)
        .map_err(|e| RailError::CaptureError(format!("sigmf meta parse: {e}")))?;

    let data_len = std::fs::metadata(data_path)
        .map_err(|e| RailError::CaptureError(format!("sigmf data stat: {e}")))?
        .len();
    if data_len % BYTES_PER_SAMPLE != 0 {
        log::warn!(
            "sigmf data file length {data_len} not a multiple of {BYTES_PER_SAMPLE}; truncating"
        );
    }
    let total_samples = data_len / BYTES_PER_SAMPLE;

    // Prefer structured SigMfMeta decoding (strict field names) but
    // fall back to loose extraction if it doesn't match — external
    // .sigmf files may omit our `rail:` fields.
    let (sample_rate_hz, center_frequency_hz, demod_mode, filter_bandwidth_hz, datetime) =
        if let Ok(meta) = serde_json::from_value::<SigMfMeta>(raw.clone()) {
            let dt = meta
                .captures
                .first()
                .map(|c| c.datetime.clone())
                .unwrap_or_default();
            (
                meta.global.sample_rate as u32,
                meta.global.center_frequency_hz,
                meta.global.demod_mode,
                meta.global.filter_bandwidth_hz,
                dt,
            )
        } else {
            let sr = raw["global"]["core:sample_rate"].as_u64().unwrap_or(0) as u32;
            let cf = raw["global"]["rail:center_frequency_hz"]
                .as_u64()
                .or_else(|| raw["captures"][0]["core:frequency"].as_u64())
                .unwrap_or(0);
            let mode = raw["global"]["rail:demod_mode"]
                .as_str()
                .unwrap_or("FM")
                .to_string();
            let bw = raw["global"]["rail:filter_bandwidth_hz"]
                .as_u64()
                .unwrap_or(200_000) as u32;
            let dt = raw["captures"][0]["core:datetime"]
                .as_str()
                .unwrap_or("")
                .to_string();
            (sr, cf, mode, bw, dt)
        };

    if sample_rate_hz == 0 {
        return Err(RailError::CaptureError(
            "sigmf meta missing or zero core:sample_rate".into(),
        ));
    }
    let datatype = raw["global"]["core:datatype"].as_str().unwrap_or("cf32_le");
    if datatype != "cf32_le" {
        return Err(RailError::CaptureError(format!(
            "unsupported SigMF datatype {datatype} (RAIL only reads cf32_le)"
        )));
    }

    Ok(ReplayInfo {
        data_path: data_path.to_path_buf(),
        meta_path,
        sample_rate_hz,
        center_frequency_hz,
        demod_mode,
        filter_bandwidth_hz,
        total_samples,
        datetime_iso8601: datetime,
    })
}

/// Backfill up to `WATERFALL_PREFILL_ROWS` FFT windows ending at
/// `sample_idx` so the waterfall shows real history immediately on
/// seek instead of a blank canvas. Oldest frame is emitted first to
/// match the live-paint scroll order (newest ends up at the top).
///
/// The reader's file cursor is left positioned at `sample_idx` on
/// success so the caller can resume normal pacing without a second
/// seek. Returns `false` only if the DSP channel has been dropped.
fn prefill_waterfall(
    reader: &mut BufReader<File>,
    dsp_tx: &mpsc::Sender<DspInput>,
    sample_idx: u64,
    sample_rate_hz: u32,
) -> bool {
    let step_samples = (sample_rate_hz as u64 * CHUNK_MS) / 1_000;
    if step_samples == 0 {
        return true;
    }
    let available = sample_idx / step_samples;
    let n_rows = (available as usize).min(WATERFALL_PREFILL_ROWS);
    if n_rows == 0 {
        // Fewer than one row of history exists before the cursor.
        // Still reposition so the caller can read from sample_idx.
        let _ = reader.seek(SeekFrom::Start(sample_idx.saturating_mul(BYTES_PER_SAMPLE)));
        return true;
    }

    let mut raw = vec![0u8; FFT_SIZE * BYTES_PER_SAMPLE as usize];
    for i in 0..n_rows {
        // age goes n_rows..=1 so the first emit is the oldest window.
        let age = (n_rows - i) as u64;
        let slot_start_samples = sample_idx.saturating_sub(age * step_samples);
        let byte_offset = slot_start_samples * BYTES_PER_SAMPLE;
        if let Err(e) = reader.seek(SeekFrom::Start(byte_offset)) {
            log::warn!("prefill seek failed at sample {slot_start_samples}: {e}");
            continue;
        }
        if let Err(e) = reader.read_exact(&mut raw) {
            log::warn!("prefill read failed at sample {slot_start_samples}: {e}");
            continue;
        }
        let mut samples: Vec<Complex<f32>> = Vec::with_capacity(FFT_SIZE);
        for pair in raw.chunks_exact(BYTES_PER_SAMPLE as usize) {
            let re = f32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
            let im = f32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]);
            samples.push(Complex::new(re, im));
        }
        if dsp_tx
            .blocking_send(DspInput::Cf32Prefill(samples))
            .is_err()
        {
            return false;
        }
    }

    let resume_offset = sample_idx.saturating_mul(BYTES_PER_SAMPLE);
    if let Err(e) = reader.seek(SeekFrom::Start(resume_offset)) {
        log::warn!("prefill resume-seek failed: {e}");
    }
    true
}

/// Convert a position in ms to an absolute sample index, clamped.
pub fn ms_to_sample_idx(ms: u64, sample_rate_hz: u32, total_samples: u64) -> u64 {
    if sample_rate_hz == 0 {
        return 0;
    }
    let idx = ms.saturating_mul(sample_rate_hz as u64) / 1_000;
    idx.min(total_samples)
}

/// Spawn the reader task. Returns a join handle tied to the spawned
/// `tokio::task` so the caller can await clean shutdown on `Stop`.
///
/// The task owns the open [`File`], paces reads by wall-clock time
/// against `sample_rate_hz`, and publishes `replay-position` events on
/// a 40 ms cadence. Closing `ctl_rx` (i.e. dropping all senders) has
/// the same effect as sending [`ReplayControl::Stop`].
pub fn spawn_replay_reader<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    info: ReplayInfo,
    dsp_tx: mpsc::Sender<DspInput>,
    mut ctl_rx: mpsc::UnboundedReceiver<ReplayControl>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let file = match File::open(&info.data_path) {
            Ok(f) => f,
            Err(e) => {
                log::error!("replay open failed: {e}");
                return;
            }
        };
        let mut reader = BufReader::with_capacity(256 * 1024, file);

        let chunk_samples: usize =
            ((info.sample_rate_hz as u64 * CHUNK_MS) / 1_000).max(1) as usize;
        let chunk_duration = Duration::from_millis(CHUNK_MS);
        let total_ms = info.duration_ms();

        let mut sample_idx: u64 = 0;
        let mut playing = true;
        let mut next_tick = Instant::now();
        let mut last_emit = Instant::now() - POSITION_EMIT_INTERVAL;

        let emit_position = |sample_idx: u64, playing: bool| {
            let position_ms = if info.sample_rate_hz > 0 {
                sample_idx.saturating_mul(1_000) / info.sample_rate_hz as u64
            } else {
                0
            };
            if let Err(e) =
                ReplayPosition::new(sample_idx, position_ms, total_ms, playing).emit(&app)
            {
                log::warn!("replay-position emit failed: {e}");
            }
        };
        emit_position(sample_idx, playing);

        loop {
            // Drain any control messages before doing any work.
            loop {
                match ctl_rx.try_recv() {
                    Ok(ReplayControl::Play) => {
                        if !playing {
                            playing = true;
                            next_tick = Instant::now();
                            emit_position(sample_idx, playing);
                        }
                    }
                    Ok(ReplayControl::Pause) => {
                        if playing {
                            playing = false;
                            emit_position(sample_idx, playing);
                        }
                    }
                    Ok(ReplayControl::Seek { sample_idx: s }) => {
                        sample_idx = s.min(info.total_samples);
                        if !prefill_waterfall(&mut reader, &dsp_tx, sample_idx, info.sample_rate_hz)
                        {
                            emit_position(sample_idx, false);
                            return;
                        }
                        next_tick = Instant::now();
                        emit_position(sample_idx, playing);
                    }
                    Ok(ReplayControl::Stop) => {
                        emit_position(sample_idx, false);
                        return;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        emit_position(sample_idx, false);
                        return;
                    }
                }
            }

            if !playing {
                // Park briefly so we keep polling the control channel
                // without spinning.
                match ctl_rx.blocking_recv() {
                    Some(ReplayControl::Play) => {
                        playing = true;
                        next_tick = Instant::now();
                        emit_position(sample_idx, playing);
                        continue;
                    }
                    Some(ReplayControl::Pause) => continue,
                    Some(ReplayControl::Seek { sample_idx: s }) => {
                        sample_idx = s.min(info.total_samples);
                        if !prefill_waterfall(&mut reader, &dsp_tx, sample_idx, info.sample_rate_hz)
                        {
                            emit_position(sample_idx, false);
                            return;
                        }
                        emit_position(sample_idx, playing);
                        continue;
                    }
                    Some(ReplayControl::Stop) | None => {
                        emit_position(sample_idx, false);
                        return;
                    }
                }
            }

            if sample_idx >= info.total_samples {
                // End of file: loop back to the start and keep playing.
                // The frontend detects the backward position jump in
                // the `replay-position` event and resets the waterfall
                // so its Y-axis stays aligned with file time.
                sample_idx = 0;
                if let Err(e) = reader.seek(SeekFrom::Start(0)) {
                    log::warn!("replay loop seek failed: {e}; stopping");
                    emit_position(sample_idx, false);
                    return;
                }
                next_tick = Instant::now();
                emit_position(sample_idx, playing);
                continue;
            }

            let want = chunk_samples.min((info.total_samples - sample_idx) as usize);
            let mut bytes = vec![0u8; want * BYTES_PER_SAMPLE as usize];
            if let Err(e) = reader.read_exact(&mut bytes) {
                log::warn!("replay read failed: {e}; stopping");
                emit_position(sample_idx, false);
                return;
            }
            let mut samples: Vec<Complex<f32>> = Vec::with_capacity(want);
            for pair in bytes.chunks_exact(BYTES_PER_SAMPLE as usize) {
                let re = f32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
                let im = f32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]);
                samples.push(Complex::new(re, im));
            }

            // Real-time pacing: sleep until `next_tick` before shipping
            // the chunk, then advance the tick by one chunk duration.
            let now = Instant::now();
            if next_tick > now {
                std::thread::sleep(next_tick - now);
            }
            next_tick += chunk_duration;

            if dsp_tx
                .blocking_send(DspInput::Cf32Shifted(samples))
                .is_err()
            {
                // DSP task exited; nothing more to do.
                emit_position(sample_idx, false);
                return;
            }
            sample_idx += want as u64;

            if last_emit.elapsed() >= POSITION_EMIT_INTERVAL {
                emit_position(sample_idx, playing);
                last_emit = Instant::now();
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_path_swaps_extension() {
        assert_eq!(
            meta_path_for(Path::new("/x/y.sigmf-data")),
            PathBuf::from("/x/y.sigmf-meta")
        );
    }

    #[test]
    fn meta_path_falls_back_when_ext_unexpected() {
        assert_eq!(
            meta_path_for(Path::new("/x/y.bin")),
            PathBuf::from("/x/y.sigmf-meta")
        );
    }

    #[test]
    fn ms_to_sample_idx_clamps() {
        assert_eq!(ms_to_sample_idx(1_000, 1_000, 500), 500);
        assert_eq!(ms_to_sample_idx(2_500, 2_000, 10_000), 5_000);
        assert_eq!(ms_to_sample_idx(0, 2_048_000, 10_000), 0);
    }
}
