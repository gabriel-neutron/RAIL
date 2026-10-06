import { create } from "zustand";

import { startScan, stopScan, type StartScanArgs } from "../ipc/commands";

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

/// Result of one `runScanSession` call. A rejection travels back as a
/// value so each call site surfaces it its own way.
export type ScanOutcome = { ok: true } | { ok: false; message: string };

type ScannerState = {
  visible: boolean;
  scanning: boolean;
  frequenciesHz: number[];
  results: ScanResult[];
  /// Current scan parameters shown in the Scanner form.
  scanConfig: ScanConfig;

  toggleVisible: () => void;
  beginScan: (frequenciesHz: number[]) => void;
  pushResult: (result: ScanResult) => void;
  endScan: () => void;
  setScanConfig: (config: ScanConfig) => void;
  /// Run one full sweep: mint the channel, decode every frame, map the
  /// results onto the reply's frequencies, and report the outcome. The
  /// caller decides how a failure is surfaced — this never logs.
  runScanSession: (args: StartScanArgs) => Promise<ScanOutcome>;
  /// Stop the running sweep. Late frames from it are discarded.
  cancelScanSession: () => Promise<void>;
};

/// Byte layout of one scan frame. See `docs/ARCHITECTURE.md` §3.3
/// (`scanChannel`) for what the two fields mean.
const SIGNAL_AVG_DB_BYTE_OFFSET = 0;
const NOISE_FLOOR_DB_BYTE_OFFSET = 4;
const LITTLE_ENDIAN = true;

/// The only place the scan wire format is read.
const decodeScanFrame = (
  buffer: ArrayBuffer,
): { signalAvgDb: number; noiseFloorDb: number } => {
  const view = new DataView(buffer);
  return {
    signalAvgDb: view.getFloat32(SIGNAL_AVG_DB_BYTE_OFFSET, LITTLE_ENDIAN),
    noiseFloorDb: view.getFloat32(NOISE_FLOOR_DB_BYTE_OFFSET, LITTLE_ENDIAN),
  };
};

/// Bumped by every session start and every cancel, so frames from a
/// superseded sweep cannot write into the results of the current one.
let generation = 0;

export const useScannerStore = create<ScannerState>((set) => ({
  visible: true,
  scanning: false,
  frequenciesHz: [],
  results: [],
  scanConfig: DEFAULT_CONFIG,

  toggleVisible: () => set((s) => ({ visible: !s.visible })),

  beginScan: (frequenciesHz) =>
    set({ scanning: true, frequenciesHz, results: [] }),

  pushResult: (result) =>
    set((s) => ({ results: [...s.results, result] })),

  endScan: () => set({ scanning: false }),

  setScanConfig: (scanConfig) => set({ scanConfig }),

  runScanSession: async (args) => {
    generation += 1;
    const session = generation;

    // The host can deliver frames before `start_scan` resolves, and the
    // frequency list only arrives with that reply. Buffer until then,
    // otherwise the first step is lost and every later result is mapped
    // one frequency short.
    let freqs: number[] | null = null;
    const pending: ArrayBuffer[] = [];
    let idx = 0;

    const consume = (buffer: ArrayBuffer, frequencies: number[]) => {
      if (idx >= frequencies.length) return;
      const { signalAvgDb, noiseFloorDb } = decodeScanFrame(buffer);
      const frequencyHz = frequencies[idx];
      idx += 1;
      useScannerStore
        .getState()
        .pushResult({ frequencyHz, signalAvgDb, noiseFloorDb });
    };

    const onFrame = (buffer: ArrayBuffer) => {
      if (session !== generation) return;
      if (freqs === null) {
        pending.push(buffer);
        return;
      }
      consume(buffer, freqs);
    };

    try {
      const reply = await startScan(args, onFrame);
      if (session !== generation) return { ok: true };
      freqs = reply.frequenciesHz;
      useScannerStore.getState().beginScan(freqs);
      for (const buffered of pending) consume(buffered, freqs);
      pending.length = 0;
      return { ok: true };
    } catch (err) {
      return { ok: false, message: String(err) };
    }
  },

  cancelScanSession: async () => {
    generation += 1;
    try {
      await stopScan();
    } catch (err) {
      console.warn("[RAIL] stopScan failed:", err);
    }
    useScannerStore.getState().endScan();
  },
}));
