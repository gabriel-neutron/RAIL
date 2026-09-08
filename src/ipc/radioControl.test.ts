import { beforeEach, describe, expect, it, vi } from "vitest";

import { createFakeClock, type FakeClock } from "../test/fakeClock";
import {
  clampGainIndex,
  clampPpm,
  createRadioControl,
  snapGainToNearest,
  type RadioCommands,
  type RadioControl,
} from "./radioControl";

const makeCommands = () => ({
  retune: vi.fn(() => Promise.resolve({ frequencyHz: 0 })),
  setMode: vi.fn(() => Promise.resolve()),
  setBandwidth: vi.fn(() => Promise.resolve()),
  setSquelch: vi.fn(() => Promise.resolve()),
  setGain: vi.fn(() => Promise.resolve()),
  setPpm: vi.fn(() => Promise.resolve()),
});

let commands: RadioCommands & ReturnType<typeof makeCommands>;
let clock: FakeClock;
let streaming: boolean;
let replaying: boolean;
let control: RadioControl;

beforeEach(() => {
  commands = makeCommands();
  clock = createFakeClock();
  streaming = true;
  replaying = false;
  control = createRadioControl({
    commands,
    clock,
    guards: {
      canControl: () => streaming,
      canTouchHardware: () => streaming && !replaying,
    },
  });
});

describe("debouncing", () => {
  it("coalesces a burst into one send carrying the last value", () => {
    control.retune(100_000_000);
    control.retune(100_100_000);
    control.retune(100_200_000);
    expect(commands.retune).not.toHaveBeenCalled();

    clock.advance(30);
    expect(commands.retune).toHaveBeenCalledTimes(1);
    expect(commands.retune).toHaveBeenCalledWith(100_200_000);
  });

  it("cancels pending timers on reset so nothing fires after teardown", () => {
    control.mode("NFM");
    expect(clock.pendingCount()).toBe(1);
    control.reset();
    clock.advance(60);
    expect(commands.setMode).not.toHaveBeenCalled();
  });

  it("still dispatches after a reset", () => {
    control.mode("NFM");
    control.reset();
    clock.advance(60);

    control.mode("AM");
    expect(clock.pendingCount()).toBe(1);
    clock.advance(60);
    expect(commands.setMode).toHaveBeenCalledTimes(1);
    expect(commands.setMode).toHaveBeenCalledWith("AM");
  });

  it("drops the value queued before a reset rather than sending it late", () => {
    control.retune(100_000_000);
    control.reset();

    control.retune(101_000_000);
    clock.advance(30);
    expect(commands.retune).toHaveBeenCalledTimes(1);
    expect(commands.retune).toHaveBeenCalledWith(101_000_000);
  });

  it("swallows a rejected send rather than leaking an unhandled rejection", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    commands.setBandwidth.mockReturnValueOnce(
      Promise.reject(new Error("stream not running")),
    );
    control.bandwidth(12_500);
    clock.advance(60);
    await Promise.resolve();
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });
});

describe("guards", () => {
  it("drops every dispatch while nothing is streaming", () => {
    streaming = false;
    control.retune(90_000_000);
    control.mode("AM");
    control.gain({ auto: true });
    clock.advance(60);
    expect(commands.retune).not.toHaveBeenCalled();
    expect(commands.setMode).not.toHaveBeenCalled();
    expect(commands.setGain).not.toHaveBeenCalled();
  });

  it("keeps demod verbs alive during replay but blocks the tuner ones", async () => {
    replaying = true;
    control.mode("NFM");
    control.bandwidth(12_500);
    control.gain({ auto: false, tenthsDb: 496 });
    control.retune(90_000_000);
    clock.advance(60);
    expect(commands.setMode).toHaveBeenCalledWith("NFM");
    expect(commands.setBandwidth).toHaveBeenCalledWith(12_500);
    expect(commands.setGain).not.toHaveBeenCalled();
    expect(commands.retune).not.toHaveBeenCalled();

    await control.ppm(12);
    expect(commands.setPpm).not.toHaveBeenCalled();
  });
});

describe("ppm", () => {
  it("clamps before sending and rejects to the caller", async () => {
    await control.ppm(9_000);
    expect(commands.setPpm).toHaveBeenCalledWith(200);

    commands.setPpm.mockReturnValueOnce(Promise.reject(new Error("nope")));
    await expect(control.ppm(4)).rejects.toThrow("nope");
  });
});

describe("policy helpers", () => {
  it("truncates and clamps a ppm correction", () => {
    expect(clampPpm(12.9)).toBe(12);
    expect(clampPpm(-12.9)).toBe(-12);
    expect(clampPpm(5_000)).toBe(200);
    expect(clampPpm(-5_000)).toBe(-200);
  });

  it("clamps a gain index into the hardware list", () => {
    const gains = [0, 87, 166, 496];
    expect(clampGainIndex(-3, gains)).toBe(0);
    expect(clampGainIndex(99, gains)).toBe(3);
    expect(clampGainIndex(2, gains)).toBe(2);
  });

  it("keeps a supported gain and takes the midpoint otherwise", () => {
    const gains = [0, 87, 166, 496];
    expect(snapGainToNearest(gains, 87)).toBe(87);
    expect(snapGainToNearest(gains, 12)).toBe(166);
  });
});
