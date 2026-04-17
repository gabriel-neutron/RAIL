import { create } from "zustand";

import { retune } from "../ipc/commands";

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
};

const RETUNE_DEBOUNCE_MS = 30;

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
  setFrequency: (frequencyHz) => {
    // Round to integer Hz — the backend `retune` command deserializes
    // `frequencyHz` as `u32`, so fractional values (e.g. from click-to-tune
    // pixel math) would be silently rejected by serde.
    const hz = Math.max(0, Math.round(frequencyHz));
    set({ frequencyHz: hz });
    scheduleRetune(hz, get().streaming);
  },
  setSampleRate: (sampleRateHz) => set({ sampleRateHz }),
  setMode: (mode) => set({ mode }),
  setBandwidth: (bandwidthHz) => set({ bandwidthHz }),
  setAutoGain: (autoGain) => set({ autoGain }),
  setGainTenthsDb: (gainTenthsDb) => set({ gainTenthsDb }),
  setAvailableGains: (availableGainsTenthsDb) => set({ availableGainsTenthsDb }),
  setPpm: (ppm) => set({ ppm }),
  setFreqUnit: (freqUnit) => set({ freqUnit }),
  setStreaming: (streaming) => set({ streaming }),
}));
