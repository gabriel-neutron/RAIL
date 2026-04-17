import { create } from "zustand";

export type DemodMode = "FM" | "AM" | "USB" | "LSB" | "CW";

export type RadioState = {
  frequencyHz: number;
  mode: DemodMode;
  bandwidthHz: number;
  autoGain: boolean;
  gainTenthsDb: number;
  availableGainsTenthsDb: number[];
  streaming: boolean;
  setFrequency: (hz: number) => void;
  setMode: (mode: DemodMode) => void;
  setBandwidth: (hz: number) => void;
  setAutoGain: (auto: boolean) => void;
  setGainTenthsDb: (tenths: number) => void;
  setAvailableGains: (gains: number[]) => void;
  setStreaming: (streaming: boolean) => void;
};

export const useRadioStore = create<RadioState>((set) => ({
  frequencyHz: 100_000_000,
  mode: "FM",
  bandwidthHz: 200_000,
  autoGain: true,
  gainTenthsDb: 0,
  availableGainsTenthsDb: [],
  streaming: false,
  setFrequency: (frequencyHz) => set({ frequencyHz }),
  setMode: (mode) => set({ mode }),
  setBandwidth: (bandwidthHz) => set({ bandwidthHz }),
  setAutoGain: (autoGain) => set({ autoGain }),
  setGainTenthsDb: (gainTenthsDb) => set({ gainTenthsDb }),
  setAvailableGains: (availableGainsTenthsDb) => set({ availableGainsTenthsDb }),
  setStreaming: (streaming) => set({ streaming }),
}));
