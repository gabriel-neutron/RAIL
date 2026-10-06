import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { setTransport } from "../ipc/transport";
import { createMockTransport, type MockTransport } from "../test/mockTransport";
import { useReplayStore } from "./replay";

const initial = useReplayStore.getState();

let mock: MockTransport;

beforeEach(() => {
  useReplayStore.setState(initial, true);
  mock = createMockTransport();
  setTransport(mock);
});

afterEach(() => {
  setTransport(null);
});

const transportCalls = () =>
  mock.calls.filter((c) => c.command === "replay_transport");

describe("togglePlay", () => {
  it("pauses a playing file", async () => {
    useReplayStore.setState({ active: true, playing: true });
    await useReplayStore.getState().togglePlay();
    expect(mock.payloadOf("replay_transport")).toEqual({
      args: { kind: "pause" },
    });
    expect(useReplayStore.getState().playing).toBe(false);
  });

  it("resumes a paused file", async () => {
    useReplayStore.setState({ active: true, playing: false });
    await useReplayStore.getState().togglePlay();
    expect(mock.payloadOf("replay_transport")).toEqual({
      args: { kind: "play" },
    });
    expect(useReplayStore.getState().playing).toBe(true);
  });

  it("does nothing without a loaded file", async () => {
    await useReplayStore.getState().togglePlay();
    expect(transportCalls()).toHaveLength(0);
  });
});

describe("seek", () => {
  it("sends an integer, non-negative position and bumps the waterfall epoch", async () => {
    useReplayStore.setState({ active: true, waterfallEpoch: 3 });
    await useReplayStore.getState().seek(1_499.6);
    expect(mock.payloadOf("replay_transport")).toEqual({
      args: { kind: "seek", positionMs: 1_500 },
    });
    expect(useReplayStore.getState().waterfallEpoch).toBe(4);
  });

  it("clamps a negative scrub to zero", async () => {
    useReplayStore.setState({ active: true });
    await useReplayStore.getState().seek(-200);
    expect(mock.payloadOf("replay_transport")).toEqual({
      args: { kind: "seek", positionMs: 0 },
    });
  });
});

describe("close", () => {
  it("tears the session down through stopStream, not a transport verb", async () => {
    useReplayStore.setState({ active: true, playing: true, waterfallEpoch: 1 });
    await useReplayStore.getState().close();
    expect(mock.payloadOf("stop_stream")).toBeUndefined();
    expect(transportCalls()).toHaveLength(0);
    expect(useReplayStore.getState().active).toBe(false);
    expect(useReplayStore.getState().waterfallEpoch).toBe(2);
  });
});

describe("openFile", () => {
  it("loads the metadata for the picked path", async () => {
    mock.nextOpenPath = "C:/captures/RAIL.sigmf-data";
    mock.reply("open_replay", { dataPath: "C:/captures/RAIL.sigmf-data" });
    await useReplayStore.getState().openFile();
    expect(mock.payloadOf("open_replay")).toEqual({
      args: { dataPath: "C:/captures/RAIL.sigmf-data" },
    });
    expect(useReplayStore.getState().active).toBe(true);
  });

  it("stays idle when the user cancels the dialog", async () => {
    mock.nextOpenPath = null;
    await useReplayStore.getState().openFile();
    expect(mock.calls).toHaveLength(0);
    expect(useReplayStore.getState().active).toBe(false);
  });
});
