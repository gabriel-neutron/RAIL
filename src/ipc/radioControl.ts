// The control seam: everything between "the user moved a knob" and
// "a command reached the backend". Debouncing, the streaming/replay
// guards and the gain/PPM clamping policy live here so components and
// stores hold none of it. See `docs/ARCHITECTURE.md` (frontend control seam).

import type { DemodModeWire, SetGainArgs } from "./commands";

/// Smallest and largest crystal correction the backend accepts.
export const MIN_PPM_CORRECTION = -200;
export const MAX_PPM_CORRECTION = 200;

const RETUNE_DEBOUNCE_MS = 30;
const COMMAND_DEBOUNCE_MS = 60;

/// Truncate-and-clamp a PPM correction. `n | 0` truncates toward zero at
/// 32 bits, which is what the text field has always accepted — rounding
/// instead would change which entries are legal.
export const clampPpm = (n: number): number =>
  Math.max(MIN_PPM_CORRECTION, Math.min(MAX_PPM_CORRECTION, n | 0));

/// Clamp a gain-slider index into the hardware-supplied list.
export const clampGainIndex = (index: number, gains: number[]): number =>
  Math.max(0, Math.min(gains.length - 1, index));

/// Pick a gain the hardware actually supports. Keeps `current` when the
/// list contains it, otherwise takes the midpoint — the sane default on
/// a first connection or a device swap.
export const snapGainToNearest = (gains: number[], current: number): number =>
  gains.includes(current) ? current : gains[Math.floor(gains.length / 2)];

/// The timer surface the seam needs, injectable so tests drive a fake.
export type Clock = {
  setTimeout: (fn: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
};

const realClock: Clock = {
  setTimeout: (fn, ms) => setTimeout(fn, ms) as unknown as number,
  clearTimeout: (id) => {
    clearTimeout(id);
  },
};

/// The subset of `ipc/commands` the seam dispatches. Structural so a
/// test can hand in plain stubs without a module mock.
export type RadioCommands = {
  retune: (frequencyHz: number) => Promise<unknown>;
  setMode: (mode: DemodModeWire) => Promise<unknown>;
  setBandwidth: (bandwidthHz: number) => Promise<unknown>;
  setSquelch: (thresholdDbfs: number | null) => Promise<unknown>;
  setGain: (args: SetGainArgs) => Promise<unknown>;
  setPpm: (ppm: number) => Promise<unknown>;
};

/// Predicates read at dispatch time. `canControl` gates the demod verbs
/// (valid during replay too); `canTouchHardware` gates the tuner verbs,
/// which a replayed file has no hardware for.
export type RadioGuards = {
  canControl: () => boolean;
  canTouchHardware: () => boolean;
};

export type RadioControlDeps = {
  commands: RadioCommands;
  guards: RadioGuards;
  clock?: Clock;
};

export type RadioControl = {
  retune: (frequencyHz: number) => void;
  mode: (mode: DemodModeWire) => void;
  bandwidth: (bandwidthHz: number) => void;
  squelch: (thresholdDbfs: number | null) => void;
  gain: (args: SetGainArgs) => void;
  /// Returns its promise so the caller can surface the failure — PPM is
  /// the one control with a visible error affordance.
  ppm: (correction: number) => Promise<void>;
  /// The hardware-reach predicate, exposed so a caller dispatching a
  /// command this seam does not own (the band-menu scan) applies the same
  /// policy instead of re-deriving it from `streaming`.
  canTouchHardware: () => boolean;
  /// Drop every pending timer and every queued value, leaving the
  /// dispatcher able to schedule again. Used by tests between cases, and
  /// available for session teardown.
  reset: () => void;
};

/// Build a dispatcher. No module-level instance is exported on purpose:
/// the debounce timers are state, and state that outlives a store cannot
/// be reset between tests.
export const createRadioControl = ({
  commands,
  guards,
  clock = realClock,
}: RadioControlDeps): RadioControl => {
  // Each debouncer registers its own cancel here. Clearing the timer ids
  // alone would leave the closure's `timer` non-null, and the
  // `if (timer !== null) return` short-circuit would wedge the verb for
  // the life of the store.
  const cancels: (() => void)[] = [];

  const debounced = <T,>(
    label: string,
    delayMs: number,
    canSend: () => boolean,
    send: (value: T) => Promise<unknown>,
  ) => {
    let timer: number | null = null;
    let pending: { value: T } | null = null;
    cancels.push(() => {
      if (timer !== null) clock.clearTimeout(timer);
      timer = null;
      pending = null;
    });
    return (value: T) => {
      pending = { value };
      if (!canSend()) return;
      if (timer !== null) return;
      timer = clock.setTimeout(() => {
        timer = null;
        const next = pending;
        pending = null;
        if (next === null) return;
        send(next.value).catch((err: unknown) => {
          // Fire-and-forget on purpose: a timer can fire after the
          // stream was torn down, and surfacing "stream not running"
          // would turn routine teardown into a user-facing error.
          console.warn(`[RAIL] ${label} failed:`, err);
        });
      }, delayMs);
    };
  };

  const { canControl, canTouchHardware } = guards;

  return {
    retune: debounced<number>("retune", RETUNE_DEBOUNCE_MS, canTouchHardware, (hz) =>
      commands.retune(hz),
    ),
    mode: debounced<DemodModeWire>(
      "set_mode",
      COMMAND_DEBOUNCE_MS,
      canControl,
      (mode) => commands.setMode(mode),
    ),
    bandwidth: debounced<number>(
      "set_bandwidth",
      COMMAND_DEBOUNCE_MS,
      canControl,
      (hz) => commands.setBandwidth(hz),
    ),
    squelch: debounced<number | null>(
      "set_squelch",
      COMMAND_DEBOUNCE_MS,
      canControl,
      (db) => commands.setSquelch(db),
    ),
    gain: debounced<SetGainArgs>(
      "set_gain",
      COMMAND_DEBOUNCE_MS,
      canTouchHardware,
      (args) => commands.setGain(args),
    ),
    ppm: async (correction) => {
      if (!canTouchHardware()) return;
      await commands.setPpm(clampPpm(correction));
    },
    canTouchHardware,
    reset: () => {
      for (const cancel of cancels) cancel();
    },
  };
};
