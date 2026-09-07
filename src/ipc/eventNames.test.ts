import { describe, expect, it } from "vitest";

import eventNames from "../../shared/ipc_event_names.json";
import * as generated from "./generated/eventNames";

// The Rust side generates its own constants from the same JSON in build.rs.
// This guards the TS half: if the committed generated file drifts from the
// shared contract, the IPC bus silently stops delivering events.
describe("generated IPC event names", () => {
  it("exports exactly the keys declared in shared/ipc_event_names.json", () => {
    expect(Object.keys(generated).sort()).toEqual(Object.keys(eventNames).sort());
  });

  it("matches the wire string for every key", () => {
    for (const [key, wire] of Object.entries(eventNames)) {
      expect(generated[key as keyof typeof generated]).toBe(wire);
    }
  });

  it("has no duplicate wire names", () => {
    const wires = Object.values(eventNames);
    expect(new Set(wires).size).toBe(wires.length);
  });
});
