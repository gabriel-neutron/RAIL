import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setTransport } from "../ipc/transport";
import { createMockTransport, type MockTransport } from "../test/mockTransport";
import { useScannerStore } from "./scanner";

const initial = useScannerStore.getState();

let mock: MockTransport;

beforeEach(() => {
  useScannerStore.setState(initial, true);
  mock = createMockTransport();
  setTransport(mock);
});

afterEach(() => {
  setTransport(null);
  vi.restoreAllMocks();
});

const frame = (signalAvgDb: number, noiseFloorDb: number): ArrayBuffer => {
  const buffer = new ArrayBuffer(8);
  const view = new DataView(buffer);
  view.setFloat32(0, signalAvgDb, true);
  view.setFloat32(4, noiseFloorDb, true);
  return buffer;
};

const ARGS = {
  startHz: 88_000_000,
  stopHz: 88_400_000,
  stepHz: 200_000,
  dwellMs: 200,
  squelchSnrDb: null,
};

const replyWith = (frequenciesHz: number[]) => {
  mock.reply("start_scan", { frequenciesHz });
};

describe("runScanSession", () => {
  it("maps frames positionally onto the reply frequencies", async () => {
    replyWith([88_000_000, 88_200_000, 88_400_000]);
    const outcome = await useScannerStore.getState().runScanSession(ARGS);
    expect(outcome).toEqual({ ok: true });

    mock.emit(0, frame(-30, -80));
    mock.emit(0, frame(-45, -81));

    expect(useScannerStore.getState().results).toEqual([
      { frequencyHz: 88_000_000, signalAvgDb: -30, noiseFloorDb: -80 },
      { frequencyHz: 88_200_000, signalAvgDb: -45, noiseFloorDb: -81 },
    ]);
    expect(useScannerStore.getState().scanning).toBe(true);
  });

  it("buffers a frame that arrives before the reply and drains it in order", async () => {
    replyWith([88_000_000, 88_200_000]);
    const pending = useScannerStore.getState().runScanSession(ARGS);

    mock.emit(0, frame(-10, -70));
    expect(useScannerStore.getState().results).toHaveLength(0);

    await pending;
    mock.emit(0, frame(-20, -71));

    expect(useScannerStore.getState().results).toEqual([
      { frequencyHz: 88_000_000, signalAvgDb: -10, noiseFloorDb: -70 },
      { frequencyHz: 88_200_000, signalAvgDb: -20, noiseFloorDb: -71 },
    ]);
  });

  it("drops frames past the end of the frequency list", async () => {
    replyWith([88_000_000]);
    await useScannerStore.getState().runScanSession(ARGS);

    mock.emit(0, frame(-30, -80));
    mock.emit(0, frame(-31, -81));

    expect(useScannerStore.getState().results).toHaveLength(1);
  });

  it("reports a rejection as an outcome without logging or scanning", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    mock.failWith("start_scan", new Error("no device"));

    const outcome = await useScannerStore.getState().runScanSession(ARGS);

    expect(outcome).toEqual({ ok: false, message: "Error: no device" });
    expect(useScannerStore.getState().scanning).toBe(false);
    expect(useScannerStore.getState().results).toHaveLength(0);
    expect(warn).not.toHaveBeenCalled();
    expect(error).not.toHaveBeenCalled();
  });

  it("ignores a late frame from a superseded session", async () => {
    replyWith([88_000_000, 88_200_000]);
    await useScannerStore.getState().runScanSession(ARGS);
    await useScannerStore.getState().runScanSession(ARGS);

    mock.emit(0, frame(-30, -80));

    expect(useScannerStore.getState().results).toHaveLength(0);
  });

  it("passes squelchSnrDb as null and never sends thresholdSnrDb", async () => {
    replyWith([88_000_000]);
    await useScannerStore.getState().runScanSession(ARGS);

    const payload = mock.payloadOf("start_scan");
    expect(payload?.args).toEqual({
      startHz: 88_000_000,
      stopHz: 88_400_000,
      stepHz: 200_000,
      dwellMs: 200,
      squelchSnrDb: null,
    });
    expect(JSON.stringify(payload)).not.toContain("thresholdSnrDb");
  });
});

describe("cancelScanSession", () => {
  it("stops the sweep once and makes later frames inert", async () => {
    replyWith([88_000_000, 88_200_000]);
    await useScannerStore.getState().runScanSession(ARGS);

    await useScannerStore.getState().cancelScanSession();

    expect(mock.payloadOf("stop_scan")).toBeUndefined();
    expect(useScannerStore.getState().scanning).toBe(false);

    mock.emit(0, frame(-30, -80));
    expect(useScannerStore.getState().results).toHaveLength(0);
  });
});
