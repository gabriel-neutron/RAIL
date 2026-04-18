import { create } from "zustand";

import {
  retune,
  setBandwidth as setBandwidthCmd,
  setMode as setModeCmd,
  setSquelch as setSquelchCmd,
  type DemodModeWire,
} from "../ipc/commands";

export type DemodMode = "FM" | "AM" | "USB" | "LSB" | "CW";

export type FreqUnit = "Hz" | "kHz" | "MHz";

/// Multiplier from a freq-unit to Hz. Exported so keyboard shortcuts can
/// read the current unit scale without duplicating the table.
export const UNIT_SCALE: Record<FreqUnit, number> = {
  Hz: 1,
  kHz: 1_000,
  MHz: 1_000_000,
};

export type RadioState = {
  frequencyHz: number;
  sampleRateHz: number;
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
  setFrequency: (hz: number) => void;
  setSampleRate: (hz: number) => void;
  setMode: (mode: DemodMode) => void;
  setBandwidth: (hz: number) => void;
  setAutoGain: (auto: boolean) => void;
  setGainTenthsDb: (tenths: number) => void;
  setAvailableGains: (gains: number[]) => void;
  setPpm: (ppm: number) => void;
  setFreqUnit: (unit: FreqUnit) => void;
  setStreaming: (streaming: boolean) => void;
  setVolume: (v: number) => void;
  setMuted: (m: boolean) => void;
  setSquelchDbfs: (db: number | null) => void;
};

const RETUNE_DEBOUNCE_MS = 30;
const COMMAND_DEBOUNCE_MS = 60;

let retuneTimer: ReturnType<typeof setTimeout> | null = null;
let pendingRetuneHz: number | null = null;

const scheduleRetune = (hz: number, streaming: boolean) => {
  pendingRetuneHz = hz;
  if (!streaming) return;
  if (retuneTimer !== null) return;
  retuneTimer = setTimeout(() => {
    retuneTimer = null;
    const target = pendingRetuneHz;
    pendingRetuneHz = null;
    if (target === null) return;
    retune(target).catch((err) => {
      console.warn("[RAIL] retune failed:", err);
    });
  }, RETUNE_DEBOUNCE_MS);
};

const makeDebouncer = <T,>(
  label: string,
  dispatch: (value: T) => Promise<unknown>,
) => {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: { value: T } | null = null;
  return (value: T, streaming: boolean) => {
    pending = { value };
    if (!streaming) return;
    if (timer !== null) return;
    timer = setTimeout(() => {
      timer = null;
      const next = pending;
      pending = null;
      if (next === null) return;
      dispatch(next.value).catch((err) => {
        console.warn(`[RAIL] ${label} failed:`, err);
      });
    }, COMMAND_DEBOUNCE_MS);
  };
};

const scheduleMode = makeDebouncer<DemodMode>("set_mode", (mode) =>
  setModeCmd(mode as DemodModeWire),
);
const scheduleBandwidth = makeDebouncer<number>("set_bandwidth", (hz) =>
  setBandwidthCmd(hz),
);
const scheduleSquelch = makeDebouncer<number | null>("set_squelch", (db) =>
  setSquelchCmd(db),
);

export const useRadioStore = create<RadioState>((set, get) => ({
  frequencyHz: 100_000_000,
  sampleRateHz: 2_048_000,
  mode: "FM",
  bandwidthHz: 200_000,
  autoGain: true,
  gainTenthsDb: 0,
  availableGainsTenthsDb: [],
  ppm: 0,
  freqUnit: "MHz",
  streaming: false,
  volume: 0.7,
  muted: false,
  squelchDbfs: null,
  setFrequency: (frequencyHz) => {
    // Round to integer Hz — the backend `retune` command deserializes
    // `frequencyHz` as `u32`, so fractional values (e.g. from click-to-tune
    // pixel math) would be silently rejected by serde.
    const hz = Math.max(0, Math.round(frequencyHz));
    set({ frequencyHz: hz });
    scheduleRetune(hz, get().streaming);
  },
  setSampleRate: (sampleRateHz) => set({ sampleRateHz }),
  setMode: (mode) => {
    set({ mode });
    // USB/LSB/CW stay frontend-only placeholders in Phase 3 — the
    // Rust side rejects them with InvalidParameter, so avoid the
    // round-trip.
    if (mode === "FM" || mode === "AM") {
      scheduleMode(mode, get().streaming);
    }
  },
  setBandwidth: (bandwidthHz) => {
    set({ bandwidthHz });
    scheduleBandwidth(bandwidthHz, get().streaming);
  },
  setAutoGain: (autoGain) => set({ autoGain }),
  setGainTenthsDb: (gainTenthsDb) => set({ gainTenthsDb }),
  setAvailableGains: (availableGainsTenthsDb) => set({ availableGainsTenthsDb }),
  setPpm: (ppm) => set({ ppm }),
  setFreqUnit: (freqUnit) => set({ freqUnit }),
  setStreaming: (streaming) => {
    set({ streaming });
    if (streaming) {
      // Re-push the demod config on stream start so Rust's default
      // (WBFM/200 kHz/squelch off) matches the UI.
      const s = get();
      if (s.mode === "FM" || s.mode === "AM") {
        scheduleMode(s.mode, true);
      }
      scheduleBandwidth(s.bandwidthHz, true);
      scheduleSquelch(s.squelchDbfs, true);
    }
  },
  setVolume: (v) => set({ volume: Math.max(0, Math.min(1, v)) }),
  setMuted: (muted) => set({ muted }),
  setSquelchDbfs: (squelchDbfs) => {
    set({ squelchDbfs });
    scheduleSquelch(squelchDbfs, get().streaming);
  },
}));
