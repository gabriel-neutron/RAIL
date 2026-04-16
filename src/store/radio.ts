import { create } from "zustand";

export type DemodMode = "FM" | "AM" | "USB" | "LSB" | "CW";

export type RadioState = {
  frequencyHz: number;
  mode: DemodMode;
  bandwidthHz: number;
  gainDb: number;
  streaming: boolean;
  setFrequency: (hz: number) => void;
  setMode: (mode: DemodMode) => void;
  setBandwidth: (hz: number) => void;
  setGain: (db: number) => void;
  setStreaming: (streaming: boolean) => void;
};

export const useRadioStore = create<RadioState>((set) => ({
  frequencyHz: 100_000_000,
  mode: "FM",
  bandwidthHz: 200_000,
  gainDb: 0,
  streaming: false,
  setFrequency: (frequencyHz) => set({ frequencyHz }),
  setMode: (mode) => set({ mode }),
  setBandwidth: (bandwidthHz) => set({ bandwidthHz }),
  setGain: (gainDb) => set({ gainDb }),
  setStreaming: (streaming) => set({ streaming }),
}));
