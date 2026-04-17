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
  channel: Channel<ArrayBuffer>,
): Promise<StartStreamReply> =>
  invoke<StartStreamReply>("start_stream", { args, channel });

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
