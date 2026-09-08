import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

import { useReplayStore } from "../../store/replay";
import { Transport } from "./index";

const initial = useReplayStore.getState();

const info = {
  dataPath: "/tmp/cap.sigmf-data",
  metaPath: "/tmp/cap.sigmf-meta",
  sampleRateHz: 2_400_000,
  centerFrequencyHz: 100_000_000,
  demodMode: "wfm",
  filterBandwidthHz: 200_000,
  totalSamples: 24_000_000,
  durationMs: 10_000,
  datetimeIso8601: "2026-01-01T00:00:00Z",
};

const makeSeekMock = () => vi.fn<(positionMs: number) => Promise<void>>();

let seek: ReturnType<typeof makeSeekMock>;

beforeEach(() => {
  seek = makeSeekMock();
  useReplayStore.setState(initial, true);
  useReplayStore.setState({ active: true, playing: true, info, seek });
});

afterEach(() => {
  useReplayStore.setState(initial, true);
});

const slider = () => screen.getByLabelText("Seek");

describe("Transport scrubbing", () => {
  it("does not seek while the user is dragging", () => {
    render(<Transport />);
    fireEvent.change(slider(), { target: { value: "1000" } });
    fireEvent.change(slider(), { target: { value: "2000" } });
    expect(seek).not.toHaveBeenCalled();
    expect(slider()).toHaveValue("2000");
  });

  it("seeks exactly once on pointer release, then resumes the clock", () => {
    render(<Transport />);
    fireEvent.change(slider(), { target: { value: "3000" } });
    fireEvent.pointerUp(window);
    expect(seek).toHaveBeenCalledTimes(1);
    expect(seek).toHaveBeenCalledWith(3000);

    act(() => {
      useReplayStore.setState({ positionMs: 4200 });
    });
    expect(slider()).toHaveValue("4200");
  });

  it("commits on pointercancel", () => {
    render(<Transport />);
    fireEvent.change(slider(), { target: { value: "1500" } });
    fireEvent.pointerCancel(window);
    expect(seek).toHaveBeenCalledTimes(1);
    expect(seek).toHaveBeenCalledWith(1500);
  });

  it("seeks once when a pointerup is immediately followed by a keyup", () => {
    render(<Transport />);
    fireEvent.change(slider(), { target: { value: "2500" } });
    fireEvent.pointerUp(window);
    fireEvent.keyUp(window, { key: "ArrowRight" });
    expect(seek).toHaveBeenCalledTimes(1);
  });

  it("renders nothing without a loaded file", () => {
    useReplayStore.setState({ active: false });
    const { container } = render(<Transport />);
    expect(container).toBeEmptyDOMElement();
  });
});
