import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  openReplay: vi.fn(() => Promise.resolve(null)),
  replayTransport: vi.fn(() => Promise.resolve()),
  stopStream: vi.fn(() => Promise.resolve()),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

import { useReplayStore } from "./replay";

const initial = useReplayStore.getState();

beforeEach(() => {
  useReplayStore.setState(initial, true);
  vi.clearAllMocks();
});

describe("togglePlay", () => {
  it("pauses a playing file", async () => {
    const { replayTransport } = await import("../ipc/commands");
    useReplayStore.setState({ active: true, playing: true });
    await useReplayStore.getState().togglePlay();
    expect(replayTransport).toHaveBeenCalledWith({ kind: "pause" });
    expect(useReplayStore.getState().playing).toBe(false);
  });

  it("resumes a paused file", async () => {
    const { replayTransport } = await import("../ipc/commands");
    useReplayStore.setState({ active: true, playing: false });
    await useReplayStore.getState().togglePlay();
    expect(replayTransport).toHaveBeenCalledWith({ kind: "play" });
    expect(useReplayStore.getState().playing).toBe(true);
  });

  it("does nothing without a loaded file", async () => {
    const { replayTransport } = await import("../ipc/commands");
    await useReplayStore.getState().togglePlay();
    expect(replayTransport).not.toHaveBeenCalled();
  });
});

describe("seek", () => {
  it("sends an integer, non-negative position and bumps the waterfall epoch", async () => {
    const { replayTransport } = await import("../ipc/commands");
    useReplayStore.setState({ active: true, waterfallEpoch: 3 });
    await useReplayStore.getState().seek(1_499.6);
    expect(replayTransport).toHaveBeenCalledWith({
      kind: "seek",
      positionMs: 1_500,
    });
    expect(useReplayStore.getState().waterfallEpoch).toBe(4);
  });

  it("clamps a negative scrub to zero", async () => {
    const { replayTransport } = await import("../ipc/commands");
    useReplayStore.setState({ active: true });
    await useReplayStore.getState().seek(-200);
    expect(replayTransport).toHaveBeenCalledWith({ kind: "seek", positionMs: 0 });
  });
});

describe("close", () => {
  it("tears the session down through stopStream, not a transport verb", async () => {
    const { replayTransport, stopStream } = await import("../ipc/commands");
    useReplayStore.setState({ active: true, playing: true, waterfallEpoch: 1 });
    await useReplayStore.getState().close();
    expect(stopStream).toHaveBeenCalledTimes(1);
    expect(replayTransport).not.toHaveBeenCalled();
    expect(useReplayStore.getState().active).toBe(false);
    expect(useReplayStore.getState().waterfallEpoch).toBe(2);
  });
});
