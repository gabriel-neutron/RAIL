import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

vi.mock("../../ipc/commands", () => ({
  retune: vi.fn(() => Promise.resolve({ frequencyHz: 0 })),
  setBandwidth: vi.fn(() => Promise.resolve()),
  setMode: vi.fn(() => Promise.resolve()),
  setSquelch: vi.fn(() => Promise.resolve()),
  setGain: vi.fn(() => Promise.resolve()),
  setPpm: vi.fn(() => Promise.resolve()),
}));

import { useRadioStore } from "../../store/radio";
import { useReplayStore } from "../../store/replay";
import { FrequencyControl } from "./index";

const initial = useRadioStore.getState();

beforeEach(() => {
  useRadioStore.setState(initial, true);
  useRadioStore.setState({ frequencyHz: 100_000_000, freqUnit: "MHz" });
  useReplayStore.setState({ active: false });
  // The store debounces IPC through setTimeout; fake timers keep those from
  // firing after the test ends.
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

const field = () => screen.getByLabelText("Center frequency in MHz");

describe("FrequencyControl draft syncing", () => {
  it("shows the store frequency", () => {
    render(<FrequencyControl />);
    expect(field()).toHaveValue("100.000000");
  });

  it("follows an external retune while unfocused", () => {
    render(<FrequencyControl />);
    act(() => {
      useRadioStore.setState({ frequencyHz: 88_500_000 });
    });
    expect(field()).toHaveValue("88.500000");
  });

  it("does not clobber in-progress typing while focused", () => {
    render(<FrequencyControl />);
    fireEvent.focus(field());
    fireEvent.change(field(), { target: { value: "144.3" } });
    act(() => {
      useRadioStore.setState({ frequencyHz: 88_500_000 });
    });
    expect(field()).toHaveValue("144.3");
  });

  it("commits the typed value on blur", () => {
    render(<FrequencyControl />);
    fireEvent.focus(field());
    fireEvent.change(field(), { target: { value: "144.3" } });
    fireEvent.blur(field());
    expect(useRadioStore.getState().frequencyHz).toBe(144_300_000);
    expect(field()).toHaveValue("144.300000");
  });

  it("reverts to the canonical value when the typed text is not a number", () => {
    render(<FrequencyControl />);
    fireEvent.focus(field());
    fireEvent.change(field(), { target: { value: "abc" } });
    fireEvent.blur(field());
    expect(useRadioStore.getState().frequencyHz).toBe(100_000_000);
    expect(field()).toHaveValue("100.000000");
  });
});
