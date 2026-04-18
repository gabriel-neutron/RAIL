import { open } from "@tauri-apps/plugin-dialog";
import { create } from "zustand";

import {
  openReplay,
  pauseReplay,
  resumeReplay,
  seekReplay,
  stopReplay,
  type ReplayInfoReply,
} from "../ipc/commands";

export type ReplayInfo = ReplayInfoReply;

/// Any position discontinuity (seek or end-of-file loop) larger than
/// this is treated as a new waterfall segment. Keep well above the
/// replay-reader emit cadence (~40 ms, see `crate::replay`) so the
/// normal forward drift between events never trips the detector.
const JUMP_RESET_MS = 250;

type ReplayState = {
  /// True while a SigMF file is loaded (playing or paused). Used to
  /// gate live-only controls and show the Transport bar.
  active: boolean;
  playing: boolean;
  info: ReplayInfo | null;
  positionMs: number;

  /// Monotonic counter that changes whenever the playhead
  /// discontinuously moves (open, seek, loop). The waterfall canvas
  /// subscribes to this and clears itself on change so the time axis
  /// reflects file time, not wall-clock emit order.
  waterfallEpoch: number;

  openFile: () => Promise<void>;
  close: () => Promise<void>;
  togglePlay: () => Promise<void>;
  seek: (positionMs: number) => Promise<void>;

  /// Fed from the `replay-position` event listener in App.tsx.
  applyPosition: (positionMs: number, playing: boolean) => void;

  /// Internal helper used by the pipeline hook after a successful
  /// `start_replay` call — fills in the metadata returned by the
  /// backend so the Transport bar can render.
  setInfo: (info: ReplayInfo | null) => void;
};

const logError = (label: string, err: unknown) => {
  console.error(`[RAIL] ${label}:`, err);
};

export const useReplayStore = create<ReplayState>((set, get) => ({
  active: false,
  playing: false,
  info: null,
  positionMs: 0,
  waterfallEpoch: 0,

  openFile: async () => {
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "SigMF data", extensions: ["sigmf-data"] }],
      });
      if (!picked || Array.isArray(picked)) return;

      // Load metadata up-front; the pipeline hook will call
      // `start_replay` once the live stream has been torn down.
      const info = await openReplay(picked);
      set((s) => ({
        active: true,
        playing: true,
        info,
        positionMs: 0,
        waterfallEpoch: s.waterfallEpoch + 1,
      }));
    } catch (err) {
      logError("open replay failed", err);
    }
  },

  close: async () => {
    if (!get().active) return;
    try {
      await stopReplay();
    } catch (err) {
      logError("stop replay failed", err);
    }
    set((s) => ({
      active: false,
      playing: false,
      info: null,
      positionMs: 0,
      waterfallEpoch: s.waterfallEpoch + 1,
    }));
  },

  togglePlay: async () => {
    const { active, playing } = get();
    if (!active) return;
    try {
      if (playing) {
        await pauseReplay();
        set({ playing: false });
      } else {
        await resumeReplay();
        set({ playing: true });
      }
    } catch (err) {
      logError("toggle replay failed", err);
    }
  },

  seek: async (positionMs) => {
    if (!get().active) return;
    try {
      await seekReplay(Math.max(0, Math.round(positionMs)));
      // Optimistic local update + epoch bump so the waterfall resets
      // immediately on the user's scrub instead of waiting for the
      // next replay-position event to fan out.
      set((s) => ({
        positionMs,
        waterfallEpoch: s.waterfallEpoch + 1,
      }));
    } catch (err) {
      logError("seek replay failed", err);
    }
  },

  applyPosition: (positionMs, playing) => {
    // Catch backend-originated jumps (end-of-file loop in particular).
    // User-initiated seeks already bumped the epoch in `seek`; the
    // redundant bump that would fire here is harmless.
    const prev = get().positionMs;
    const jumped = Math.abs(positionMs - prev) > JUMP_RESET_MS;
    set((s) => ({
      positionMs,
      playing,
      waterfallEpoch: jumped ? s.waterfallEpoch + 1 : s.waterfallEpoch,
    }));
  },

  setInfo: (info) => set({ info }),
}));
