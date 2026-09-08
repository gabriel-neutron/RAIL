import { beforeEach, describe, expect, it, vi } from "vitest";

import contract from "../../shared/ipc_events.json";
import * as generated from "./generated/events";
import { subscribeIpcEvent } from "./events";

type ListenCallback = (evt: { payload: unknown }) => void;

const listen = vi.hoisted(() =>
  vi.fn<(name: string, cb: ListenCallback) => Promise<() => void>>(),
);
vi.mock("@tauri-apps/api/event", () => ({ listen }));

// The Rust half of the contract comes out of the same generator and is diffed
// by the codegen CI job. This guards the TS half: if the committed generated
// file drifts from the shared contract, the bus silently stops delivering.
describe("generated IPC event contract", () => {
  it("exports a constant for exactly the keys declared in the contract", () => {
    const constants = Object.keys(generated).filter((k) =>
      k.startsWith("EVENT_"),
    );
    expect(constants.sort()).toEqual(Object.keys(contract).sort());
  });

  it("matches the wire string for every key", () => {
    for (const [key, entry] of Object.entries(contract)) {
      expect(generated[key as keyof typeof generated]).toBe(entry.name);
    }
  });

  it("has no duplicate wire names", () => {
    const wires = Object.values(contract).map((entry) => entry.name);
    expect(new Set(wires).size).toBe(wires.length);
  });
});

describe("subscribeIpcEvent", () => {
  beforeEach(() => {
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("forwards the wire name and unwraps the payload for device-status", async () => {
    const handler = vi.fn();
    await subscribeIpcEvent(generated.EVENT_DEVICE_STATUS, handler);

    expect(listen).toHaveBeenCalledWith("device-status", expect.any(Function));
    listen.mock.calls[0][1]({ payload: { connected: false, error: "gone" } });
    expect(handler).toHaveBeenCalledWith({ connected: false, error: "gone" });
  });

  it("forwards the wire name and unwraps the payload for scan-step", async () => {
    const handler = vi.fn();
    await subscribeIpcEvent(generated.EVENT_SCAN_STEP, handler);

    expect(listen).toHaveBeenCalledWith("scan-step", expect.any(Function));
    listen.mock.calls[0][1]({ payload: { frequencyHz: 100_000_000 } });
    expect(handler).toHaveBeenCalledWith({ frequencyHz: 100_000_000 });
  });

  it("returns the unlisten function from listen", async () => {
    const unlisten = vi.fn();
    listen.mockResolvedValue(unlisten);
    await expect(
      subscribeIpcEvent(generated.EVENT_SIGNAL_LEVEL, vi.fn()),
    ).resolves.toBe(unlisten);
  });
});
