import { invoke } from "@tauri-apps/api/core";

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

export const ping = (): Promise<string> => invoke<string>("ping");

export const checkDevice = (): Promise<DeviceInfo> =>
  invoke<DeviceInfo>("check_device");
