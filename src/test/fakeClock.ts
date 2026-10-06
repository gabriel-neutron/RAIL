// Deterministic stand-in for the debounce timers in `ipc/radioControl`.
// Vitest's fake timers work too, but an injected clock keeps the seam's
// tests free of global state and of the Windows libuv teardown hazard.

import type { Clock } from "../ipc/radioControl";

export type FakeClock = Clock & {
  /// Run every callback whose delay has elapsed after `ms` more time.
  advance: (ms: number) => void;
  /// Timers scheduled and not yet fired or cancelled.
  pendingCount: () => number;
};

export const createFakeClock = (): FakeClock => {
  const pending = new Map<number, { fn: () => void; dueAt: number }>();
  let now = 0;
  let nextId = 1;

  return {
    setTimeout: (fn: () => void, ms: number) => {
      const id = nextId++;
      pending.set(id, { fn, dueAt: now + ms });
      return id;
    },
    clearTimeout: (id: number) => {
      pending.delete(id);
    },
    advance: (ms: number) => {
      now += ms;
      for (const [id, timer] of [...pending.entries()]) {
        if (timer.dueAt <= now) {
          pending.delete(id);
          timer.fn();
        }
      }
    },
    pendingCount: () => pending.size,
  };
};
