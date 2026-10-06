import { create } from "zustand";

import * as commands from "../ipc/commands";
import { type SignalClassificationPayload } from "../ipc/generated/events";
import {
  clampGainIndex,
  clampPpm,
  createRadioControl,
  snapGainToNearest,
} from "../ipc/radioControl";
import { useReplayStore } from "./replay";

/// Every demodulation mode the backend accepts, in selector order.
/// Kept in sync with `parse_mode` in `src-tauri/src/ipc/commands.rs`.
export const DEMOD_MODES = ["FM", "NFM", "AM", "USB", "LSB", "CW"] as const;

export type DemodMode = (typeof DEMOD_MODES)[number];

/// Narrow an untrusted mode string (SigMF metadata, bookmarks) to a
/// `DemodMode`, or `null` when it is not one this build knows.
export const parseDemodMode = (value: string): DemodMode | null =>
  (DEMOD_MODES as readonly string[]).includes(value)
    ? (value as DemodMode)
    : null;

export type FreqUnit = "Hz" | "kHz" | "MHz";

/// Multiplier from a freq-unit to Hz. Exported so keyboard shortcuts can
/// read the current unit scale without duplicating the table.
export const UNIT_SCALE: Record<FreqUnit, number> = {
  Hz: 1,
  kHz: 1_000,
  MHz: 1_000_000,
};

/// Signal strength snapshot (dBFS). Fed by the `signal-level` event.
export type SignalLevel = {
  currentDbfs: number;
  peakDbfs: number;
};

/// Waterfall zoom range. Frontend-only crop of the FFT frame —
/// backend keeps a constant FFT size. Continuous (not stepped) so
/// the scroll-wheel handler can multiply by a smooth factor.
export const ZOOM_MIN = 1;
export const ZOOM_MAX = 64;

export type RadioState = {
  frequencyHz: number;
  sampleRateHz: number;
  /// FFT length N of the streamed frames; the frame length the waterfall crops.
  fftSize: number;
  mode: DemodMode;
  bandwidthHz: number;
  autoGain: boolean;
  gainTenthsDb: number;
  availableGainsTenthsDb: number[];
  ppm: number;
  freqUnit: FreqUnit;
  streaming: boolean;
  volume: number;
  muted: boolean;
  /// Squelch threshold in dBFS. `null` = gate disabled.
  squelchDbfs: number | null;
  /// Frontend waterfall/spectrum zoom factor. 1 = full fs span.
  zoom: number;
  /// Latest dBFS level snapshot from the backend; `null` before the
  /// first event arrives (e.g. no stream yet).
  signalLevel: SignalLevel | null;
  /// Latest signal classification from the backend; `null` when no signal
  /// is above the noise floor or no stream is running.
  classification: SignalClassificationPayload | null;
  /// True when a live device is attached and reachable — a stream is
  /// running and no replay has taken over. The predicate is defined once,
  /// in the control seam; this is the store's window onto it.
  canTouchHardware: () => boolean;
  setFrequency: (hz: number) => void;
  /// Mirror a frequency the backend has already tuned to. Updates the
  /// display only — no retune is scheduled, because the hardware is
  /// already there.
  syncFrequencyFromBackend: (hz: number) => void;
  setSampleRate: (hz: number) => void;
  setFftSize: (fftSize: number) => void;
  setMode: (mode: DemodMode) => void;
  setBandwidth: (hz: number) => void;
  setAutoGain: (auto: boolean) => void;
  setGainTenthsDb: (tenths: number) => void;
  /// Adopt the hardware-supplied gain list, snapping the current pick
  /// onto it when the device does not offer that exact value.
  setAvailableGains: (gains: number[]) => void;
  /// Pick a gain by slider index, clamped to the hardware list, and push
  /// it when the radio is in manual gain.
  selectGainIndex: (index: number) => void;
  setPpm: (ppm: number) => void;
  /// Clamp, store and push a crystal correction. Rejects to the caller so
  /// the PPM field can render the backend's complaint.
  applyPpm: (ppm: number) => Promise<void>;
  setFreqUnit: (unit: FreqUnit) => void;
  setStreaming: (streaming: boolean) => void;
  setVolume: (v: number) => void;
  setMuted: (m: boolean) => void;
  setSquelchDbfs: (db: number | null) => void;
  setZoom: (zoom: number) => void;
  setSignalLevel: (level: SignalLevel | null) => void;
  setClassification: (c: SignalClassificationPayload | null) => void;
  /// When `true`, a confirmed classifier result automatically selects
  /// the demodulation mode on each classification event. Off by default.
  autoApplyMode: boolean;
  setAutoApplyMode: (v: boolean) => void;
  /// When `false`, signal-classification events are ignored and the
  /// suggested-mode badge is hidden. On by default.
  classifierEnabled: boolean;
  setClassifierEnabled: (v: boolean) => void;
};

/// Reference bandwidth for each mode — used to rescale the squelch threshold
/// when switching modes so the gate position stays constant in SNR terms.
/// See `docs/DSP.md` §6 (NFM/WBFM squelch note).
const SQUELCH_REF_BW_HZ: Record<DemodMode, number> = {
  FM: 200_000,
  NFM: 12_500,
  AM: 10_000,
  USB: 2_700,
  LSB: 2_700,
  CW: 500,
};

export const useRadioStore = create<RadioState>((set, get) => {
  // Constructed inside the store creator so the debounce timers live and
  // die with the store — no module-level singleton to leak across tests.
  const control = createRadioControl({
    commands,
    guards: {
      canControl: () => get().streaming,
      canTouchHardware: () =>
        get().streaming && !useReplayStore.getState().active,
    },
  });

  return {
    canTouchHardware: control.canTouchHardware,
    frequencyHz: 100_000_000,
    sampleRateHz: 2_048_000,
    fftSize: 8192,
    mode: "FM",
    bandwidthHz: 200_000,
    autoGain: true,
    gainTenthsDb: 0,
    availableGainsTenthsDb: [],
    ppm: 0,
    freqUnit: "MHz",
    streaming: false,
    volume: 0.1,
    muted: false,
    squelchDbfs: null,
    zoom: 1,
    signalLevel: null,
    classification: null,
    autoApplyMode: false,
    classifierEnabled: true,
    setFrequency: (frequencyHz) => {
      // Replay sessions are locked to the capture's center frequency; the
      // backend would reject any retune with `InvalidParameter` anyway, so
      // we drop the change at the source to keep the UI honest.
      if (useReplayStore.getState().active) return;
      // Round to integer Hz — the backend `retune` command deserializes
      // `frequencyHz` as `u32`, so fractional values (e.g. from click-to-tune
      // pixel math) would be silently rejected by serde.
      const hz = Math.max(0, Math.round(frequencyHz));
      set({ frequencyHz: hz });
      control.retune(hz);
    },
    syncFrequencyFromBackend: (frequencyHz) =>
      set({ frequencyHz: Math.max(0, Math.round(frequencyHz)) }),
    setSampleRate: (sampleRateHz) => set({ sampleRateHz }),
    setFftSize: (fftSize) => set({ fftSize }),
    setMode: (mode) => {
      const prev = get().mode;
      set({ mode });
      control.mode(mode);
      // Rescale the active squelch threshold to keep the SNR gate constant
      // when moving between modes with different reference bandwidths.
      // See docs/DSP.md §6 (squelch note).
      const squelch = get().squelchDbfs;
      if (squelch !== null && Number.isFinite(squelch) && prev !== mode) {
        const offset = 10 * Math.log10(SQUELCH_REF_BW_HZ[mode] / SQUELCH_REF_BW_HZ[prev]);
        const rescaled = Math.max(-100, Math.min(0, squelch + offset));
        set({ squelchDbfs: rescaled });
        control.squelch(rescaled);
      }
    },
    setBandwidth: (bandwidthHz) => {
      set({ bandwidthHz });
      control.bandwidth(bandwidthHz);
    },
    setAutoGain: (autoGain) => {
      set({ autoGain });
      control.gain(
        autoGain ? { auto: true } : { auto: false, tenthsDb: get().gainTenthsDb },
      );
    },
    setGainTenthsDb: (gainTenthsDb) => set({ gainTenthsDb }),
    setAvailableGains: (availableGainsTenthsDb) => {
      set({ availableGainsTenthsDb });
      if (availableGainsTenthsDb.length === 0) return;
      set({
        gainTenthsDb: snapGainToNearest(
          availableGainsTenthsDb,
          get().gainTenthsDb,
        ),
      });
    },
    selectGainIndex: (index) => {
      const gains = get().availableGainsTenthsDb;
      if (gains.length === 0) return;
      const tenths = gains[clampGainIndex(index, gains)];
      set({ gainTenthsDb: tenths });
      if (!get().autoGain) control.gain({ auto: false, tenthsDb: tenths });
    },
    setPpm: (ppm) => set({ ppm: clampPpm(ppm) }),
    applyPpm: async (ppm) => {
      const clamped = clampPpm(ppm);
      set({ ppm: clamped });
      await control.ppm(clamped);
    },
    setFreqUnit: (freqUnit) => set({ freqUnit }),
    setStreaming: (streaming) => {
      set({ streaming });
      if (streaming) {
        // Re-push the demod config on stream start so Rust's default
        // (WBFM/200 kHz/squelch off) matches the UI.
        const s = get();
        control.mode(s.mode);
        control.bandwidth(s.bandwidthHz);
        control.squelch(s.squelchDbfs);
      }
    },
    setVolume: (v) => set({ volume: Math.max(0, Math.min(1, v)) }),
    setMuted: (muted) => set({ muted }),
    setSquelchDbfs: (squelchDbfs) => {
      set({ squelchDbfs });
      control.squelch(squelchDbfs);
    },
    setZoom: (zoom) => {
      if (!Number.isFinite(zoom)) return;
      const clamped = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, zoom));
      set({ zoom: clamped });
    },
    setSignalLevel: (signalLevel) => set({ signalLevel }),
    setClassification: (classification) => set({ classification }),
    setAutoApplyMode: (autoApplyMode) => set({ autoApplyMode }),
    setClassifierEnabled: (classifierEnabled) => {
      set({ classifierEnabled });
      if (!classifierEnabled) set({ classification: null });
    },
  };
});
