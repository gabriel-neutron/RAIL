//! Emit-interval stats for DSP → frontend paths. Built only with
//! `--features profile`. At runtime, set **`RAIL_PROFILE=1`** and e.g.
//! **`RUST_LOG=rail_perf=info`** (or `RUST_LOG=info`) to print periodic
//! summaries without editing source. See **`docs/PERF.md`** for the full
//! profiling runbook.

#[cfg(feature = "profile")]
mod imp {
    use std::collections::VecDeque;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    pub(super) fn enabled() -> bool {
        static E: OnceLock<bool> = OnceLock::new();
        *E.get_or_init(|| {
            std::env::var("RAIL_PROFILE")
                .map(|v| v == "1")
                .unwrap_or(false)
        })
    }

    struct Ring {
        label: &'static str,
        last: Option<Instant>,
        samples: VecDeque<u64>,
        last_log: Instant,
    }

    impl Ring {
        const CAP: usize = 256;

        fn new(label: &'static str) -> Self {
            Self {
                label,
                last: None,
                samples: VecDeque::new(),
                last_log: Instant::now(),
            }
        }

        fn record_interval(&mut self) {
            let now = Instant::now();
            if let Some(prev) = self.last.replace(now) {
                let ns = now.duration_since(prev).as_nanos() as u64;
                while self.samples.len() >= Self::CAP {
                    self.samples.pop_front();
                }
                self.samples.push_back(ns);
            }
        }

        fn maybe_log(&mut self) {
            if self.last_log.elapsed() < Duration::from_secs(4) {
                return;
            }
            self.last_log = Instant::now();
            if self.samples.is_empty() {
                return;
            }
            let mut v: Vec<u64> = self.samples.iter().copied().collect();
            v.sort_unstable();
            let n = v.len();
            let p95_idx = ((n as f64 * 0.95).floor() as usize).min(n.saturating_sub(1));
            let p50 = v[n / 2];
            let p95 = v[p95_idx];
            let sum: u128 = v.iter().map(|&x| x as u128).sum();
            let avg = (sum / n as u128) as u64;
            log::info!(
                target: "rail_perf",
                "{}: n={} avg_us={:.2} p50_us={:.2} p95_us={:.2}",
                self.label,
                n,
                avg as f64 / 1000.0,
                p50 as f64 / 1000.0,
                p95 as f64 / 1000.0,
            );
        }
    }

    static WATERFALL: OnceLock<Mutex<Ring>> = OnceLock::new();
    static AUDIO: OnceLock<Mutex<Ring>> = OnceLock::new();
    static SIGNAL_LEVEL: OnceLock<Mutex<Ring>> = OnceLock::new();

    pub fn record_waterfall_emit_interval() {
        if !enabled() {
            return;
        }
        let ring = WATERFALL.get_or_init(|| Mutex::new(Ring::new("waterfall_channel_send")));
        if let Ok(mut g) = ring.lock() {
            g.record_interval();
            g.maybe_log();
        }
    }

    pub fn record_audio_emit_interval() {
        if !enabled() {
            return;
        }
        let ring = AUDIO.get_or_init(|| Mutex::new(Ring::new("audio_channel_send")));
        if let Ok(mut g) = ring.lock() {
            g.record_interval();
            g.maybe_log();
        }
    }

    pub fn record_signal_level_emit_interval() {
        if !enabled() {
            return;
        }
        let ring = SIGNAL_LEVEL.get_or_init(|| Mutex::new(Ring::new("signal_level_json_emit")));
        if let Ok(mut g) = ring.lock() {
            g.record_interval();
            g.maybe_log();
        }
    }
}

#[cfg(feature = "profile")]
pub use imp::{
    record_audio_emit_interval, record_signal_level_emit_interval, record_waterfall_emit_interval,
};

#[cfg(not(feature = "profile"))]
#[inline]
pub fn record_waterfall_emit_interval() {}

#[cfg(not(feature = "profile"))]
#[inline]
pub fn record_audio_emit_interval() {}

#[cfg(not(feature = "profile"))]
#[inline]
pub fn record_signal_level_emit_interval() {}
