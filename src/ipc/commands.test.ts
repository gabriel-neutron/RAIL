import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  addBookmark,
  finalizeCapture,
  retune,
  saveScreenshot,
  screenshotSuggestion,
  setGain,
  setSquelch,
  startIqCapture,
  stopAudioCapture,
  stopIqCapture,
  stopScan,
  stopStream,
} from "./commands";
import { setTransport } from "./transport";
import { createMockTransport, type MockTransport } from "../test/mockTransport";

let mock: MockTransport;

beforeEach(() => {
  mock = createMockTransport();
  setTransport(mock);
});

afterEach(() => {
  setTransport(null);
});

describe("the { args } envelope", () => {
  it("wraps every parameterized command in a single args object", async () => {
    await retune(100_000_000);
    expect(mock.payloadOf("retune")).toEqual({
      args: { frequencyHz: 100_000_000 },
    });

    await setSquelch(null);
    expect(mock.payloadOf("set_squelch")).toEqual({
      args: { thresholdDbfs: null },
    });

    await setGain({ auto: false, tenthsDb: 496 });
    expect(mock.payloadOf("set_gain")).toEqual({
      args: { auto: false, tenthsDb: 496 },
    });

    await finalizeCapture("a", "b");
    expect(mock.payloadOf("finalize_capture")).toEqual({
      args: { src: "a", dst: "b" },
    });
  });

  it("wraps start_iq_capture too — Rust reads it as a StartIqCaptureArgs struct", async () => {
    await startIqCapture("ADS-B");
    expect(mock.payloadOf("start_iq_capture")).toEqual({
      args: { signalTypeGuess: "ADS-B" },
    });
  });

  it("sends no payload at all for the parameterless commands", async () => {
    await stopStream();
    await stopAudioCapture();
    await stopIqCapture();
    await screenshotSuggestion();
    await stopScan();
    for (const command of [
      "stop_stream",
      "stop_audio_capture",
      "stop_iq_capture",
      "screenshot_suggestion",
      "stop_scan",
    ]) {
      expect(mock.payloadOf(command)).toBeUndefined();
    }
  });
});

describe("payload normalisation", () => {
  it("sends null rather than undefined for absent optionals", async () => {
    await addBookmark("Air band", 118_000_000);
    expect(mock.payloadOf("add_bookmark")).toEqual({
      args: {
        name: "Air band",
        frequencyHz: 118_000_000,
        mode: null,
        bandwidthHz: null,
      },
    });

    await startIqCapture();
    expect(mock.payloadOf("start_iq_capture")).toEqual({
      args: { signalTypeGuess: null },
    });
  });

  it("widens the screenshot bytes to a plain array — Rust takes Vec<u8>", async () => {
    await saveScreenshot("out.png", new Uint8Array([1, 2, 255]));
    expect(mock.payloadOf("save_screenshot")).toEqual({
      args: { dst: "out.png", pngBytes: [1, 2, 255] },
    });
  });
});

describe("an unconfigured transport", () => {
  it("throws instead of silently dropping the command", () => {
    setTransport(null);
    expect(() => stopStream()).toThrow("[RAIL] IPC transport not configured");
  });
});
