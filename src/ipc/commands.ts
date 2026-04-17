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
