import { create } from "zustand";

export type ScanResult = {
  frequencyHz: number;
  signalAvgDb: number;
  noiseFloorDb: number;
};

export type ScanConfig = {
  startHz: number;
  stopHz: number;
  stepHz: number;
  dwellMs: number;
  thresholdSnrDb: number;
};

const DEFAULT_CONFIG: ScanConfig = {
  startHz: 87_500_000,
  stopHz: 108_000_000,
  stepHz: 200_000,
  dwellMs: 200,
  thresholdSnrDb: 10,
};

type ScannerState = {
  visible: boolean;
  scanning: boolean;
  frequenciesHz: number[];
  results: ScanResult[];
  /// Current scan parameters shown in the Scanner form.
  scanConfig: ScanConfig;
  /// Incremented each time `setScanConfig` is called so the Scanner
  /// component can detect external updates (e.g. band-menu clicks)
  /// and reset its editing state without interfering with user typing.
  scanConfigSeq: number;

  toggleVisible: () => void;
  beginScan: (frequenciesHz: number[]) => void;
  pushResult: (result: ScanResult) => void;
  endScan: () => void;
  setScanConfig: (config: ScanConfig) => void;
};

export const useScannerStore = create<ScannerState>((set) => ({
  visible: true,
  scanning: false,
  frequenciesHz: [],
  results: [],
  scanConfig: DEFAULT_CONFIG,
  scanConfigSeq: 0,

  toggleVisible: () => set((s) => ({ visible: !s.visible })),

  beginScan: (frequenciesHz) =>
    set({ scanning: true, frequenciesHz, results: [] }),

  pushResult: (result) =>
    set((s) => ({ results: [...s.results, result] })),

  endScan: () => set({ scanning: false }),

  setScanConfig: (scanConfig) =>
    set((s) => ({ scanConfig, scanConfigSeq: s.scanConfigSeq + 1 })),
}));
