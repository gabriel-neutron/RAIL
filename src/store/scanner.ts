import { create } from "zustand";

export type ScanResult = {
  frequencyHz: number;
  peakDbfs: number;
};

type ScannerState = {
  visible: boolean;
  scanning: boolean;
  frequenciesHz: number[];
  results: ScanResult[];

  toggleVisible: () => void;
  beginScan: (frequenciesHz: number[]) => void;
  pushResult: (result: ScanResult) => void;
  endScan: () => void;
};

export const useScannerStore = create<ScannerState>((set) => ({
  visible: false,
  scanning: false,
  frequenciesHz: [],
  results: [],

  toggleVisible: () => set((s) => ({ visible: !s.visible })),

  beginScan: (frequenciesHz) =>
    set({ scanning: true, frequenciesHz, results: [] }),

  pushResult: (result) =>
    set((s) => ({ results: [...s.results, result] })),

  endScan: () => set({ scanning: false }),
}));
