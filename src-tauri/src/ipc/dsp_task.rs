//! DSP worker task driven by a streaming or replay session.
//!
//! Hosts [`DspTaskCtx`] and [`spawn_dsp_task`] — the audio/waterfall
//! emit loop that `start_stream` and `start_replay` both drive. Kept
//! in its own module so the tuning/lifecycle commands in
//! [`super::commands`] stay short.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytemuck::cast_slice;
use num_complex::Complex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime};
use tokio::sync::mpsc;

use crate::capture::sigmf::SigMfStreamWriter;
use crate::capture::wav::WavStreamWriter;
use crate::dsp::classifier;
use crate::dsp::demod::{DemodChain, DemodControl};
use crate::dsp::input::DspInput;
use crate::dsp::waterfall::{apply_fs4_shift, iq_u8_to_complex, FrameBuilder};
use crate::error::RailError;
use crate::hardware::stream::{IqCanceler, DEFAULT_USB_BUF_LEN};
use crate::ipc::capture_cmd::{AudioStopInfo, CaptureControl, IqStopInfo};
use crate::ipc::events::{SignalClassification, SignalLevel};
use crate::perf_emit::{
    record_audio_emit_interval, record_signal_level_emit_interval, record_waterfall_emit_interval,
};

/// FFT size (bins). Gives 250 Hz/bin at 2.048 MHz sample rate.
/// See `docs/DSP.md` §2.
pub(crate) const FFT_SIZE: usize = 8192;

/// Minimum interval between waterfall frames emitted to the frontend
/// (~25 fps cap, `docs/DSP.md` §3).
const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(40);

/// Minimum interval between `signal-level` JSON events (~25 Hz).
/// Same cadence as waterfall frames — keeps meter and spectrum in step.
const MIN_LEVEL_EMIT_INTERVAL: Duration = Duration::from_millis(40);

/// Decay per emission for the backend peak-hold used by the signal
/// meter. 1 dB/frame at ~25 Hz gives a ~4 s fall from a peak to the
/// noise floor, which reads naturally in the UI.
const PEAK_DECAY_DB_PER_EMIT: f32 = 1.0;

/// Target audio chunk length emitted to the frontend (~40 ms @ 44.1 kHz).
/// The resampler output is variable, so this is a *minimum* drain size.
pub(crate) const AUDIO_CHUNK_SAMPLES: usize = 1764;

/// Minimum interval between `signal-classification` JSON events (~2 Hz).
/// Classification is heavier than signal-level; badge updates need no
/// sub-second cadence.
const MIN_CLASSIFY_INTERVAL: Duration = Duration::from_millis(500);

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_dsp_task<R: Runtime>(
    app: AppHandle<R>,
    iq_rx: mpsc::Receiver<DspInput>,
    waterfall_channel: Channel<InvokeResponseBody>,
    audio_channel: Channel<InvokeResponseBody>,
    control_rx: mpsc::UnboundedReceiver<DemodControl>,
    capture_rx: mpsc::UnboundedReceiver<CaptureControl>,
    canceler: Option<IqCanceler>,
    sample_rate_hz: u32,
    latest_dbfs_bits: Arc<AtomicU32>,
    center_hz_bits: Arc<AtomicU32>,
    max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let mut ctx = DspTaskCtx::<R>::new(
            app,
            sample_rate_hz,
            latest_dbfs_bits,
            center_hz_bits,
            max_dbfs_per_bin,
        );
        ctx.run(
            iq_rx,
            waterfall_channel,
            audio_channel,
            control_rx,
            capture_rx,
            canceler,
        );
    })
}

struct DspTaskCtx<R: Runtime> {
    app: AppHandle<R>,
    builder: FrameBuilder,
    chain: DemodChain,
    /// IQ samples converted to complex, already `fs/4`-shifted.
    shifted: Vec<Complex<f32>>,
    /// Samples awaiting enough length for a full FFT frame.
    fft_pending: Vec<Complex<f32>>,
    /// Reused scratch for one FFT frame (capacity `FFT_SIZE`).
    /// Avoids a per-frame allocation in `emit_waterfall_frames`.
    frame_buf: Vec<Complex<f32>>,
    /// Resampled audio awaiting enough length to ship a chunk.
    audio_pending: Vec<f32>,
    /// Reused scratch for one audio chunk (capacity
    /// `AUDIO_CHUNK_SAMPLES`). Same reasoning as `frame_buf`.
    audio_frame_buf: Vec<f32>,
    phase_idx: u32,
    last_emit: Instant,
    peak_dbfs: f32,
    last_level_emit: Instant,
    sample_rate_hz: u32,
    /// `Some` while an audio recording is in progress.
    audio_writer: Option<WavStreamWriter>,
    /// `Some` while an IQ recording is in progress.
    iq_writer: Option<SigMfStreamWriter>,
    /// Latest raw-IQ RMS in dBFS (raw f32 bits). Used by `emit_signal_level`
    /// to drive the signal meter. Computed from the fs/4-shifted IQ buffer
    /// *before* demodulation; using raw IQ avoids the ~20–30 dB noise floor
    /// inflation that FM/AM discriminators introduce on thermal noise.
    latest_dbfs_bits: Arc<AtomicU32>,
    /// Current centre frequency in Hz (plain u32, not float bits).
    /// Updated atomically by [`retune`] so the classifier always uses
    /// the live-tuned frequency. See `docs/TIMELINE.md` Phase 10.
    center_hz_bits: Arc<AtomicU32>,
    /// Timestamp of the last `signal-classification` event emit.
    last_classify_emit: Instant,
    /// Last FFT spectrum snapshot retained for classification.
    /// Populated in `emit_waterfall_frames`; consumed in `emit_classification`.
    last_spectrum: Vec<f32>,
    /// Per-bin peak dBFS accumulator shared with the scanner task.
    /// Updated element-wise on every waterfall frame: each bin holds the
    /// maximum spectral power observed since the scanner last reset it.
    /// The scanner resets this at the end of the settle window and reads
    /// it at the end of the dwell window. See `crate::scanner`.
    max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
}

/// Move exactly `n` items from `pending` into `scratch`, reusing
/// `scratch`'s allocation. Panics (debug only) if `pending.len() < n`.
///
/// This is the allocation-free replacement for `drain(..n).collect()` on
/// each FFT frame. Extracted so the reuse property can be asserted without
/// a Tauri runtime.
#[inline]
fn take_frame<T: Copy>(pending: &mut Vec<T>, scratch: &mut Vec<T>, n: usize) {
    debug_assert!(pending.len() >= n);
    scratch.clear();
    scratch.extend(pending.drain(..n));
}

impl<R: Runtime> DspTaskCtx<R> {
    fn new(
        app: AppHandle<R>,
        sample_rate_hz: u32,
        latest_dbfs_bits: Arc<AtomicU32>,
        center_hz_bits: Arc<AtomicU32>,
        max_dbfs_per_bin: Arc<Mutex<Vec<f32>>>,
    ) -> Self {
        Self {
            app,
            builder: FrameBuilder::new(FFT_SIZE),
            chain: DemodChain::new(sample_rate_hz as f32),
            shifted: Vec::with_capacity(DEFAULT_USB_BUF_LEN as usize / 2),
            fft_pending: Vec::with_capacity(FFT_SIZE * 2),
            frame_buf: Vec::with_capacity(FFT_SIZE),
            audio_pending: Vec::with_capacity(AUDIO_CHUNK_SAMPLES * 2),
            audio_frame_buf: Vec::with_capacity(AUDIO_CHUNK_SAMPLES),
            phase_idx: 0,
            last_emit: Instant::now() - MIN_EMIT_INTERVAL,
            peak_dbfs: f32::NEG_INFINITY,
            last_level_emit: Instant::now() - MIN_LEVEL_EMIT_INTERVAL,
            sample_rate_hz,
            audio_writer: None,
            iq_writer: None,
            latest_dbfs_bits,
            center_hz_bits,
            last_classify_emit: Instant::now() - MIN_CLASSIFY_INTERVAL,
            last_spectrum: Vec::with_capacity(FFT_SIZE),
            max_dbfs_per_bin,
        }
    }

    fn run(
        &mut self,
        mut iq_rx: mpsc::Receiver<DspInput>,
        waterfall_channel: Channel<InvokeResponseBody>,
        audio_channel: Channel<InvokeResponseBody>,
        mut control_rx: mpsc::UnboundedReceiver<DemodControl>,
        mut capture_rx: mpsc::UnboundedReceiver<CaptureControl>,
        canceler: Option<IqCanceler>,
    ) {
        while let Some(input) = iq_rx.blocking_recv() {
            while let Ok(msg) = control_rx.try_recv() {
                self.chain.apply(msg);
            }
            while let Ok(msg) = capture_rx.try_recv() {
                self.handle_capture(msg);
            }

            // Prefill is a special short-circuit path: one FFT window
            // per message, waterfall-only, no audio, no rate limit.
            // Used by `crate::replay` to backfill history on seek.
            if let DspInput::Cf32Prefill(samples) = input {
                if !self.emit_prefill_frame(&samples, &waterfall_channel, canceler.as_ref()) {
                    return;
                }
                continue;
            }

            match input {
                DspInput::RtlU8(chunk) => {
                    if chunk.len() % 2 != 0 {
                        log::warn!("discarding odd-length IQ chunk: {} bytes", chunk.len());
                        continue;
                    }
                    let n_complex = chunk.len() / 2;
                    self.shifted.resize(n_complex, Complex::new(0.0, 0.0));
                    if let Err(e) = iq_u8_to_complex(&chunk, &mut self.shifted) {
                        log::warn!("IQ conversion failed: {e}");
                        continue;
                    }
                    self.phase_idx = apply_fs4_shift(&mut self.shifted, self.phase_idx);
                }
                DspInput::Cf32Shifted(samples) => {
                    // Replay: samples come straight from a .sigmf-data
                    // file that was written downstream of the fs/4
                    // mixer. Skip both conversion and the shift — the
                    // data is already in the same form `self.shifted`
                    // would have after the RtlU8 branch runs.
                    self.shifted = samples;
                }
                DspInput::Cf32Prefill(_) => unreachable!("handled above"),
            }

            // Mirror shifted cf32 to the SigMF writer (if recording)
            // before any other fan-out — `self.shifted` is the same
            // buffer the waterfall FFT consumes, so the IQ file stays
            // phase-continuous with what the user sees.
            if let Some(w) = self.iq_writer.as_mut() {
                if let Err(e) = w.append_shifted(&self.shifted) {
                    log::warn!("iq writer failed, stopping recording: {e}");
                    self.iq_writer = None;
                }
            }

            // Update scanner power readout from raw IQ (before demod).
            // Raw IQ RMS is a direct measure of RF energy in the tuned
            // bandwidth; demodulated audio RMS is ~20–30 dB higher on
            // thermal noise due to discriminator noise shaping, causing
            // false positives in the scanner. See docs/DSP.md §5.
            self.latest_dbfs_bits.store(
                compute_iq_rms_dbfs(&self.shifted).to_bits(),
                Ordering::Relaxed,
            );

            if !self.emit_waterfall_frames(&waterfall_channel, canceler.as_ref()) {
                return;
            }

            let before = self.audio_pending.len();
            let rms_dbfs = self.chain.process(&self.shifted, &mut self.audio_pending);
            if let Some(w) = self.audio_writer.as_mut() {
                if let Err(e) = w.append(&self.audio_pending[before..]) {
                    log::warn!("audio writer failed, stopping recording: {e}");
                    self.audio_writer = None;
                }
            }
            if !self.emit_audio_chunks(&audio_channel, canceler.as_ref()) {
                return;
            }

            self.emit_signal_level(rms_dbfs);
            self.emit_classification();
        }

        log::debug!("dsp task exiting: iq sender dropped");
    }

    fn handle_capture(&mut self, msg: CaptureControl) {
        match msg {
            CaptureControl::StartAudio {
                path,
                sample_rate_hz,
                reply,
            } => {
                let result = if self.audio_writer.is_some() {
                    Err(RailError::CaptureError(
                        "audio recording already in progress".into(),
                    ))
                } else {
                    match WavStreamWriter::create(&path, sample_rate_hz) {
                        Ok(w) => {
                            self.audio_writer = Some(w);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                };
                let _ = reply.send(result);
            }
            CaptureControl::StopAudio { reply } => {
                let result = match self.audio_writer.take() {
                    Some(w) => {
                        let path = w.path().to_path_buf();
                        let sample_rate_hz = w.sample_rate_hz();
                        match w.finalize() {
                            Ok(samples) => Ok(AudioStopInfo {
                                path,
                                samples,
                                sample_rate_hz,
                            }),
                            Err(e) => Err(e),
                        }
                    }
                    None => Err(RailError::CaptureError(
                        "no audio recording in progress".into(),
                    )),
                };
                let _ = reply.send(result);
            }
            CaptureControl::StartIq {
                meta_path,
                data_path,
                params,
                reply,
            } => {
                let result = if self.iq_writer.is_some() {
                    Err(RailError::CaptureError(
                        "IQ recording already in progress".into(),
                    ))
                } else {
                    match SigMfStreamWriter::create(&meta_path, &data_path, params) {
                        Ok(w) => {
                            self.iq_writer = Some(w);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                };
                let _ = reply.send(result);
            }
            CaptureControl::StopIq { reply } => {
                let result = match self.iq_writer.take() {
                    Some(w) => {
                        let meta_path = w.meta_path().to_path_buf();
                        let data_path = w.data_path().to_path_buf();
                        let sample_rate_hz = self.sample_rate_hz;
                        match w.finalize() {
                            Ok(samples) => Ok(IqStopInfo {
                                meta_path,
                                data_path,
                                samples,
                                sample_rate_hz,
                            }),
                            Err(e) => Err(e),
                        }
                    }
                    None => Err(RailError::CaptureError(
                        "no IQ recording in progress".into(),
                    )),
                };
                let _ = reply.send(result);
            }
        }
    }

    fn emit_signal_level(&mut self, rms_dbfs: f32) {
        if self.last_level_emit.elapsed() < MIN_LEVEL_EMIT_INTERVAL {
            return;
        }
        let current = if rms_dbfs.is_finite() {
            rms_dbfs
        } else {
            -120.0
        };
        self.peak_dbfs = if self.peak_dbfs.is_finite() {
            (self.peak_dbfs - PEAK_DECAY_DB_PER_EMIT).max(current)
        } else {
            current
        };
        if let Err(e) = SignalLevel::new(current, self.peak_dbfs).emit(&self.app) {
            log::warn!("signal-level emit failed: {e}");
        } else {
            record_signal_level_emit_interval();
        }
        self.last_level_emit = Instant::now();
    }

    /// Classify the current signal and emit a `signal-classification` event.
    ///
    /// Rate-limited to `MIN_CLASSIFY_INTERVAL` (~2 Hz). Skips silently when
    /// no spectrum snapshot is available yet. Output contract per
    /// `docs/SIGNALS.md §5.4`.
    fn emit_classification(&mut self) {
        if self.last_classify_emit.elapsed() < MIN_CLASSIFY_INTERVAL {
            return;
        }
        if self.last_spectrum.is_empty() {
            return;
        }
        let center_hz = u64::from(self.center_hz_bits.load(Ordering::Relaxed));
        let result = classifier::classify(
            &self.last_spectrum,
            &self.shifted,
            self.sample_rate_hz,
            center_hz,
        );
        // Always emit so the frontend can clear green/show yellow priors.
        let event = SignalClassification {
            confirmed: result.confirmed,
            candidates: result.candidates,
            reason: result.reason,
        };
        if let Err(e) = event.emit(&self.app) {
            log::warn!("signal-classification emit failed: {e}");
        }
        self.last_classify_emit = Instant::now();
    }

    /// One-shot FFT + waterfall emit for a prefill window. Skips the
    /// rate limiter (`MIN_EMIT_INTERVAL`) that `emit_waterfall_frames`
    /// uses, and does not touch `fft_pending` / `last_emit` so a
    /// prefill burst doesn't starve the subsequent live-replay emits.
    /// Returns `false` if the channel is gone, matching the contract
    /// of `emit_waterfall_frames`.
    fn emit_prefill_frame(
        &mut self,
        samples: &[Complex<f32>],
        channel: &Channel<InvokeResponseBody>,
        canceler: Option<&IqCanceler>,
    ) -> bool {
        if samples.len() != FFT_SIZE {
            log::warn!(
                "prefill chunk has {} samples (expected {}); skipping",
                samples.len(),
                FFT_SIZE
            );
            return true;
        }
        match self.builder.process_shifted(samples) {
            Ok(spectrum) => {
                let bytes: &[u8] = cast_slice(spectrum);
                if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
                    log::warn!("waterfall channel send failed during prefill: {e}");
                    if let Some(c) = canceler {
                        c.cancel();
                    }
                    return false;
                }
                record_waterfall_emit_interval();
            }
            Err(e) => log::warn!("prefill frame build failed: {e}"),
        }
        true
    }

    fn emit_waterfall_frames(
        &mut self,
        channel: &Channel<InvokeResponseBody>,
        canceler: Option<&IqCanceler>,
    ) -> bool {
        self.fft_pending.extend_from_slice(&self.shifted);

        while self.fft_pending.len() >= FFT_SIZE {
            let drop_frame = self.last_emit.elapsed() < MIN_EMIT_INTERVAL;
            take_frame(&mut self.fft_pending, &mut self.frame_buf, FFT_SIZE);

            if drop_frame {
                continue;
            }

            match self.builder.process_shifted(&self.frame_buf) {
                Ok(spectrum) => {
                    // Snapshot for the classifier (runs at a lower rate than
                    // waterfall emit — see `emit_classification`).
                    self.last_spectrum.clear();
                    self.last_spectrum.extend_from_slice(spectrum);

                    // Per-bin peak accumulator for the scanner. try_lock so
                    // the DSP task never stalls if the scanner holds the lock
                    // briefly to reset or read.
                    if let Ok(mut acc) = self.max_dbfs_per_bin.try_lock() {
                        if acc.len() != spectrum.len() {
                            acc.resize(spectrum.len(), f32::NEG_INFINITY);
                        }
                        for (a, &s) in acc.iter_mut().zip(spectrum.iter()) {
                            if s > *a {
                                *a = s;
                            }
                        }
                    }

                    let bytes: &[u8] = cast_slice(spectrum);
                    // `Vec<u8>` allocation here is forced by Tauri's
                    // `InvokeResponseBody::Raw(Vec<u8>)` API — leave as-is
                    // unless upstream exposes a borrowed variant.
                    if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
                        log::warn!("waterfall channel send failed: {e}; cancelling reader");
                        if let Some(c) = canceler {
                            c.cancel();
                        }
                        return false;
                    }
                    record_waterfall_emit_interval();
                    self.last_emit = Instant::now();
                }
                Err(e) => {
                    log::warn!("frame build failed: {e}");
                }
            }
        }
        true
    }

    fn emit_audio_chunks(
        &mut self,
        channel: &Channel<InvokeResponseBody>,
        canceler: Option<&IqCanceler>,
    ) -> bool {
        while self.audio_pending.len() >= AUDIO_CHUNK_SAMPLES {
            take_frame(
                &mut self.audio_pending,
                &mut self.audio_frame_buf,
                AUDIO_CHUNK_SAMPLES,
            );
            let bytes: &[u8] = cast_slice(&self.audio_frame_buf);
            // `Vec<u8>` allocation here is forced by Tauri's
            // `InvokeResponseBody::Raw(Vec<u8>)` API — leave as-is
            // unless upstream exposes a borrowed variant.
            if let Err(e) = channel.send(InvokeResponseBody::Raw(bytes.to_vec())) {
                log::warn!("audio channel send failed: {e}; cancelling reader");
                if let Some(c) = canceler {
                    c.cancel();
                }
                return false;
            }
            record_audio_emit_interval();
        }
        true
    }
}

/// Compute the RMS magnitude of a block of complex IQ samples as dBFS.
///
/// `rms = sqrt( mean( |s[i]|² ) )`, then `20·log₁₀(rms)`.
/// Returns `f32::NEG_INFINITY` when `samples` is empty or all-zero.
/// Called every DSP cycle to update the scanner's power readout with a
/// true RF-energy measure (see `latest_dbfs_bits` in [`DspTaskCtx`]).
fn compute_iq_rms_dbfs(samples: &[Complex<f32>]) -> f32 {
    if samples.is_empty() {
        return f32::NEG_INFINITY;
    }
    let sum_sq: f32 = samples.iter().map(|s| s.norm_sqr()).sum();
    let rms = (sum_sq / samples.len() as f32).sqrt();
    if rms > 0.0 {
        20.0 * rms.log10()
    } else {
        f32::NEG_INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_frame_reuses_scratch_capacity() {
        // Seed pending with three full FFT frames worth of samples,
        // then drain them one at a time and confirm the scratch Vec
        // never has to grow past its initial `FFT_SIZE` capacity.
        let mut pending: Vec<Complex<f32>> = (0..FFT_SIZE * 3)
            .map(|i| Complex::new(i as f32, 0.0))
            .collect();
        let mut scratch: Vec<Complex<f32>> = Vec::with_capacity(FFT_SIZE);
        let initial_cap = scratch.capacity();
        assert_eq!(initial_cap, FFT_SIZE);

        for _ in 0..3 {
            take_frame(&mut pending, &mut scratch, FFT_SIZE);
            assert_eq!(scratch.len(), FFT_SIZE);
            assert_eq!(
                scratch.capacity(),
                initial_cap,
                "frame_buf must be reused, not reallocated"
            );
        }
        assert!(pending.is_empty());
    }

    #[test]
    fn take_frame_reuses_audio_scratch_capacity() {
        let mut pending: Vec<f32> = (0..AUDIO_CHUNK_SAMPLES * 2).map(|i| i as f32).collect();
        let mut scratch: Vec<f32> = Vec::with_capacity(AUDIO_CHUNK_SAMPLES);
        let initial_cap = scratch.capacity();

        for _ in 0..2 {
            take_frame(&mut pending, &mut scratch, AUDIO_CHUNK_SAMPLES);
            assert_eq!(scratch.len(), AUDIO_CHUNK_SAMPLES);
            assert_eq!(
                scratch.capacity(),
                initial_cap,
                "audio_frame_buf must be reused, not reallocated"
            );
        }
        assert!(pending.is_empty());
    }
}
