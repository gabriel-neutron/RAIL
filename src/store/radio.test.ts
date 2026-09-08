import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  retune: vi.fn(() => Promise.resolve({ frequencyHz: 0 })),
  setBandwidth: vi.fn(() => Promise.resolve()),
  setMode: vi.fn(() => Promise.resolve()),
  setSquelch: vi.fn(() => Promise.resolve()),
}));

import {
  DEMOD_MODES,
  parseDemodMode,
  useRadioStore,
  ZOOM_MAX,
  ZOOM_MIN,
} from "./radio";
import { useReplayStore } from "./replay";

const initial = useRadioStore.getState();

beforeEach(() => {
  useRadioStore.setState(initial, true);
  useReplayStore.setState({ active: false });
  vi.clearAllMocks();
  // The store debounces IPC calls through setTimeout; fake timers keep those
  // from firing after the test ends.
  vi.useFakeTimers();
});

afterEach(() => {
  // Must be restored: leaving the clock mocked trips a libuv assertion on
  // Windows once the run finishes.
  vi.useRealTimers();
});

describe("setFrequency", () => {
  it("rounds to integer Hz — the backend deserializes frequencyHz as u32", () => {
    useRadioStore.getState().setFrequency(100_000_000.7);
    expect(useRadioStore.getState().frequencyHz).toBe(100_000_001);
  });

  it("never goes negative", () => {
    useRadioStore.getState().setFrequency(-5_000);
    expect(useRadioStore.getState().frequencyHz).toBe(0);
  });

  it("is a no-op during replay, which is locked to the capture's center", () => {
    useReplayStore.setState({ active: true });
    const before = useRadioStore.getState().frequencyHz;
    useRadioStore.getState().setFrequency(88_500_000);
    expect(useRadioStore.getState().frequencyHz).toBe(before);
  });
});

describe("syncFrequencyFromBackend", () => {
  it("updates the display without scheduling a retune", async () => {
    const { retune } = await import("../ipc/commands");
    useRadioStore.setState({ streaming: true });
    useRadioStore.getState().syncFrequencyFromBackend(88_500_000.4);
    vi.runAllTimers();
    expect(useRadioStore.getState().frequencyHz).toBe(88_500_000);
    expect(retune).not.toHaveBeenCalled();
  });

  it("still lets setFrequency drive a retune", async () => {
    const { retune } = await import("../ipc/commands");
    useRadioStore.setState({ streaming: true });
    useRadioStore.getState().setFrequency(88_500_000);
    vi.runAllTimers();
    expect(retune).toHaveBeenCalledWith(88_500_000);
  });
});

describe("setMode squelch rescaling", () => {
  it("leaves squelch untouched when it is off", () => {
    useRadioStore.setState({ mode: "FM", squelchDbfs: null });
    useRadioStore.getState().setMode("NFM");
    expect(useRadioStore.getState().squelchDbfs).toBeNull();
  });

  it("lowers the threshold when moving to a narrower reference bandwidth", () => {
    // FM (200 kHz) -> NFM (12.5 kHz) is 10*log10(12500/200000) ≈ -12.04 dB.
    useRadioStore.setState({ mode: "FM", squelchDbfs: -40 });
    useRadioStore.getState().setMode("NFM");
    expect(useRadioStore.getState().squelchDbfs).toBeCloseTo(-52.04, 2);
  });

  it("clamps the rescaled threshold into [-100, 0] dBFS", () => {
    useRadioStore.setState({ mode: "FM", squelchDbfs: -95 });
    useRadioStore.getState().setMode("CW");
    expect(useRadioStore.getState().squelchDbfs).toBe(-100);

    useRadioStore.setState({ mode: "CW", squelchDbfs: -2 });
    useRadioStore.getState().setMode("FM");
    expect(useRadioStore.getState().squelchDbfs).toBe(0);
  });

  it("does not rescale when the mode is unchanged", () => {
    useRadioStore.setState({ mode: "AM", squelchDbfs: -40 });
    useRadioStore.getState().setMode("AM");
    expect(useRadioStore.getState().squelchDbfs).toBe(-40);
  });
});

describe("setZoom", () => {
  it("clamps to the supported range", () => {
    useRadioStore.getState().setZoom(1000);
    expect(useRadioStore.getState().zoom).toBe(ZOOM_MAX);
    useRadioStore.getState().setZoom(0.01);
    expect(useRadioStore.getState().zoom).toBe(ZOOM_MIN);
  });

  it("ignores non-finite input rather than poisoning the spectrum math", () => {
    useRadioStore.getState().setZoom(8);
    useRadioStore.getState().setZoom(Number.NaN);
    expect(useRadioStore.getState().zoom).toBe(8);
  });
});

describe("setVolume", () => {
  it("clamps to [0, 1]", () => {
    useRadioStore.getState().setVolume(2);
    expect(useRadioStore.getState().volume).toBe(1);
    useRadioStore.getState().setVolume(-1);
    expect(useRadioStore.getState().volume).toBe(0);
  });
});

describe("setClassifierEnabled", () => {
  it("drops any stale classification when disabled", () => {
    useRadioStore.setState({
      classification: {
        confirmed: "NFM",
        candidates: ["NFM", "AM"],
        reason: "bw 12.5 kHz, env var 0.31",
      },
      classifierEnabled: true,
    });
    useRadioStore.getState().setClassifierEnabled(false);
    expect(useRadioStore.getState().classification).toBeNull();
  });
});

describe("parseDemodMode", () => {
  it("accepts every mode the selector offers", () => {
    for (const mode of DEMOD_MODES) {
      expect(parseDemodMode(mode)).toBe(mode);
    }
  });

  it("rejects anything else rather than producing a mode with no reference bandwidth", () => {
    expect(parseDemodMode("SSTV")).toBeNull();
    expect(parseDemodMode("")).toBeNull();
    expect(parseDemodMode("fm")).toBeNull();
  });
});

describe("setStreaming", () => {
  it("pushes the adopted replay mode instead of clobbering it back to FM", async () => {
    const { setBandwidth, setMode } = await import("../ipc/commands");
    // What useWaterfall does after start_replay returns an NFM sidecar:
    // the store is still idle, so these writes are local only.
    useRadioStore.getState().setMode("NFM");
    useRadioStore.getState().setBandwidth(12_500);

    useRadioStore.getState().setStreaming(true);
    vi.runAllTimers();

    expect(setMode).toHaveBeenCalledWith("NFM");
    expect(setBandwidth).toHaveBeenCalledWith(12_500);
  });

  it("still re-pushes the user's pre-stream selection on a live start", async () => {
    const { setMode } = await import("../ipc/commands");
    useRadioStore.getState().setMode("USB");
    useRadioStore.getState().setStreaming(true);
    vi.runAllTimers();
    expect(setMode).toHaveBeenCalledWith("USB");
  });
});
