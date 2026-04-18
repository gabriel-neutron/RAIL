/// Capture state + user-flow glue for the menu-driven recorder.
///
/// Three capture kinds are supported (see `docs/SIGNALS.md`):
/// - Waterfall screenshot → `canvas.toBlob` → dialog → `save_screenshot`.
/// - Audio recording      → Rust opens a temp WAV, streams samples, dialog
///                          on Stop, then `finalize_capture` or
///                          `discard_capture`.
/// - IQ recording         → same as audio but with a SigMF meta+data pair.

import { save } from "@tauri-apps/plugin-dialog";
import { create } from "zustand";

import {
  discardCapture,
  finalizeCapture,
  finalizeIqCapture,
  saveScreenshot,
  screenshotSuggestion,
  startAudioCapture,
  startIqCapture,
  stopAudioCapture,
  stopIqCapture,
} from "../ipc/commands";

type ScreenshotProvider = () => Promise<Blob | null>;

type CaptureState = {
  recordingAudio: boolean;
  recordingIq: boolean;
  screenshotProvider: ScreenshotProvider | null;

  setScreenshotProvider: (fn: ScreenshotProvider | null) => void;

  startAudio: () => Promise<void>;
  stopAudioWithSave: () => Promise<void>;
  startIq: () => Promise<void>;
  stopIqWithSave: () => Promise<void>;
  saveScreenshot: () => Promise<void>;
};

const logError = (label: string, err: unknown) => {
  console.error(`[RAIL] ${label}:`, err);
};

/// Swap `.sigmf-data` for `.sigmf-meta` on a path the user picked from
/// the native dialog. Both files need to live next to each other for a
/// SigMF reader to pick them up.
const deriveMetaPath = (dataPath: string): string => {
  const lower = dataPath.toLowerCase();
  if (lower.endsWith(".sigmf-data")) {
    return `${dataPath.slice(0, -".sigmf-data".length)}.sigmf-meta`;
  }
  // User removed the extension or picked something unexpected — fall
  // back to appending the meta extension.
  return `${dataPath}.sigmf-meta`;
};

export const useCaptureStore = create<CaptureState>((set, get) => ({
  recordingAudio: false,
  recordingIq: false,
  screenshotProvider: null,

  setScreenshotProvider: (fn) => set({ screenshotProvider: fn }),

  startAudio: async () => {
    if (get().recordingAudio) return;
    try {
      await startAudioCapture();
      set({ recordingAudio: true });
    } catch (err) {
      logError("start audio capture failed", err);
    }
  },

  stopAudioWithSave: async () => {
    if (!get().recordingAudio) return;
    let info;
    try {
      info = await stopAudioCapture();
    } catch (err) {
      logError("stop audio capture failed", err);
      set({ recordingAudio: false });
      return;
    }
    set({ recordingAudio: false });

    try {
      const dst = await save({
        defaultPath: info.suggestedName,
        filters: [{ name: "WAV", extensions: ["wav"] }],
      });
      if (!dst) {
        await discardCapture([info.tempPath]);
        return;
      }
      await finalizeCapture(info.tempPath, dst);
    } catch (err) {
      logError("save audio dialog failed", err);
      await discardCapture([info.tempPath]).catch(() => undefined);
    }
  },

  startIq: async () => {
    if (get().recordingIq) return;
    try {
      await startIqCapture();
      set({ recordingIq: true });
    } catch (err) {
      logError("start IQ capture failed", err);
    }
  },

  stopIqWithSave: async () => {
    if (!get().recordingIq) return;
    let info;
    try {
      info = await stopIqCapture();
    } catch (err) {
      logError("stop IQ capture failed", err);
      set({ recordingIq: false });
      return;
    }
    set({ recordingIq: false });

    try {
      const dst = await save({
        defaultPath: info.suggestedName,
        filters: [{ name: "SigMF data", extensions: ["sigmf-data"] }],
      });
      if (!dst) {
        await discardCapture([info.tempDataPath, info.tempMetaPath]);
        return;
      }
      const dstData = dst.toLowerCase().endsWith(".sigmf-data")
        ? dst
        : `${dst}.sigmf-data`;
      const dstMeta = deriveMetaPath(dstData);
      await finalizeIqCapture(
        info.tempMetaPath,
        info.tempDataPath,
        dstMeta,
        dstData,
      );
    } catch (err) {
      logError("save IQ dialog failed", err);
      await discardCapture([info.tempDataPath, info.tempMetaPath]).catch(
        () => undefined,
      );
    }
  },

  saveScreenshot: async () => {
    const provider = get().screenshotProvider;
    if (!provider) {
      logError("save screenshot", "no provider registered");
      return;
    }
    let blob: Blob | null;
    try {
      blob = await provider();
    } catch (err) {
      logError("screenshot provider failed", err);
      return;
    }
    if (!blob) return;

    let suggestedName = "RAIL_screenshot.png";
    try {
      suggestedName = (await screenshotSuggestion()).suggestedName;
    } catch (err) {
      logError("screenshot suggestion failed", err);
    }

    try {
      const dst = await save({
        defaultPath: suggestedName,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      if (!dst) return;
      const bytes = new Uint8Array(await blob.arrayBuffer());
      await saveScreenshot(dst, bytes);
    } catch (err) {
      logError("save screenshot failed", err);
    }
  },
}));
