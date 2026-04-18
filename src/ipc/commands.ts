import { Channel, invoke } from "@tauri-apps/api/core";

export type DeviceInfo = {
  index: number;
  name: string;
};

export type RailErrorKind =
  | "DeviceNotFound"
  | "DeviceOpenFailed"
  | "StreamError"
  | "DspError"
  | "CaptureError"
  | "InvalidParameter";

export type RailError = {
  kind: RailErrorKind;
  message?: string;
};

export type StartStreamReply = {
  fftSize: number;
  sampleRateHz: number;
  frequencyHz: number;
  availableGainsTenthsDb: number[];
  audioSampleRateHz: number;
  audioChunkSamples: number;
};

export type StartStreamArgs = {
  frequencyHz: number;
  sampleRateHz?: number;
};

export type SetGainArgs = {
  auto: boolean;
  tenthsDb?: number;
};

export const ping = (): Promise<string> => invoke<string>("ping");

export const checkDevice = (): Promise<DeviceInfo> =>
  invoke<DeviceInfo>("check_device");

export const startStream = (
  args: StartStreamArgs,
  waterfallChannel: Channel<ArrayBuffer>,
  audioChannel: Channel<ArrayBuffer>,
): Promise<StartStreamReply> =>
  invoke<StartStreamReply>("start_stream", {
    args,
    waterfallChannel,
    audioChannel,
  });

export const stopStream = (): Promise<void> => invoke<void>("stop_stream");

export const setGain = (args: SetGainArgs): Promise<void> =>
  invoke<void>("set_gain", { args });

export const availableGains = (): Promise<number[]> =>
  invoke<number[]>("available_gains");

export type RetuneReply = {
  frequencyHz: number;
};

export const retune = (frequencyHz: number): Promise<RetuneReply> =>
  invoke<RetuneReply>("retune", { args: { frequencyHz } });

export const setPpm = (ppm: number): Promise<void> =>
  invoke<void>("set_ppm", { args: { ppm } });

export type DemodModeWire = "FM" | "AM" | "USB" | "LSB" | "CW";

export const setMode = (mode: DemodModeWire): Promise<void> =>
  invoke<void>("set_mode", { args: { mode } });

export const setBandwidth = (bandwidthHz: number): Promise<void> =>
  invoke<void>("set_bandwidth", { args: { bandwidthHz } });

/// `null` disables the gate.
export const setSquelch = (thresholdDbfs: number | null): Promise<void> =>
  invoke<void>("set_squelch", { args: { thresholdDbfs } });

export type Bookmark = {
  id: string;
  name: string;
  frequencyHz: number;
  createdAt: number;
};

export const listBookmarks = (): Promise<Bookmark[]> =>
  invoke<Bookmark[]>("list_bookmarks");

export const addBookmark = (
  name: string,
  frequencyHz: number,
): Promise<Bookmark> =>
  invoke<Bookmark>("add_bookmark", { args: { name, frequencyHz } });

export const removeBookmark = (id: string): Promise<void> =>
  invoke<void>("remove_bookmark", { args: { id } });

export const replaceBookmarks = (
  bookmarks: Bookmark[],
): Promise<Bookmark[]> =>
  invoke<Bookmark[]>("replace_bookmarks", { args: { bookmarks } });

/* -------- Capture (screenshot / audio / IQ) -------- */

export type StartAudioCaptureReply = {
  tempPath: string;
  suggestedName: string;
};

export const startAudioCapture = (): Promise<StartAudioCaptureReply> =>
  invoke<StartAudioCaptureReply>("start_audio_capture");

export type StopAudioCaptureReply = {
  tempPath: string;
  suggestedName: string;
  frequencyHz: number;
  mode: string;
  durationMs: number;
};

export const stopAudioCapture = (): Promise<StopAudioCaptureReply> =>
  invoke<StopAudioCaptureReply>("stop_audio_capture");

export type StartIqCaptureReply = {
  tempMetaPath: string;
  tempDataPath: string;
  suggestedName: string;
};

export const startIqCapture = (): Promise<StartIqCaptureReply> =>
  invoke<StartIqCaptureReply>("start_iq_capture");

export type StopIqCaptureReply = {
  tempMetaPath: string;
  tempDataPath: string;
  suggestedName: string;
  frequencyHz: number;
  durationMs: number;
};

export const stopIqCapture = (): Promise<StopIqCaptureReply> =>
  invoke<StopIqCaptureReply>("stop_iq_capture");

export const finalizeCapture = (src: string, dst: string): Promise<void> =>
  invoke<void>("finalize_capture", { args: { src, dst } });

export const finalizeIqCapture = (
  srcMeta: string,
  srcData: string,
  dstMeta: string,
  dstData: string,
): Promise<void> =>
  invoke<void>("finalize_iq_capture", {
    args: { srcMeta, srcData, dstMeta, dstData },
  });

export const discardCapture = (paths: string[]): Promise<void> =>
  invoke<void>("discard_capture", { args: { paths } });

export const screenshotSuggestion = (): Promise<{ suggestedName: string }> =>
  invoke<{ suggestedName: string }>("screenshot_suggestion");

export const saveScreenshot = (
  dst: string,
  pngBytes: Uint8Array,
): Promise<void> =>
  invoke<void>("save_screenshot", {
    args: { dst, pngBytes: Array.from(pngBytes) },
  });

/* -------- Replay (IQ file playback) -------- */

export type ReplayInfoReply = {
  dataPath: string;
  metaPath: string;
  sampleRateHz: number;
  centerFrequencyHz: number;
  demodMode: string;
  filterBandwidthHz: number;
  totalSamples: number;
  durationMs: number;
  datetimeIso8601: string;
};

export type StartReplayReply = {
  fftSize: number;
  sampleRateHz: number;
  frequencyHz: number;
  audioSampleRateHz: number;
  audioChunkSamples: number;
  info: ReplayInfoReply;
};

export const openReplay = (dataPath: string): Promise<ReplayInfoReply> =>
  invoke<ReplayInfoReply>("open_replay", { args: { dataPath } });

export const startReplay = (
  dataPath: string,
  waterfallChannel: Channel<ArrayBuffer>,
  audioChannel: Channel<ArrayBuffer>,
): Promise<StartReplayReply> =>
  invoke<StartReplayReply>("start_replay", {
    args: { dataPath },
    waterfallChannel,
    audioChannel,
  });

export const pauseReplay = (): Promise<void> => invoke<void>("pause_replay");

export const resumeReplay = (): Promise<void> => invoke<void>("resume_replay");

export const seekReplay = (positionMs: number): Promise<void> =>
  invoke<void>("seek_replay", { args: { positionMs } });

export const stopReplay = (): Promise<void> => invoke<void>("stop_replay");
