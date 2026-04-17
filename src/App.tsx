import { useCallback, useEffect, useState } from "react";
import {
  checkDevice,
  ping,
  stopStream,
  type DeviceInfo,
  type RailError,
} from "./ipc/commands";
import { subscribeDeviceStatus } from "./ipc/events";
import FrequencyControl from "./components/FrequencyControl";
import GainControl from "./components/GainControl";
import MenuBar from "./components/MenuBar";
import PpmControl from "./components/PpmControl";
import Waterfall from "./components/Waterfall";
import useKeyboardTuning from "./hooks/useKeyboardTuning";
import "./App.css";

type DeviceState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "found"; device: DeviceInfo }
  | { status: "missing"; message: string };

const isRailError = (value: unknown): value is RailError => {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    typeof (value as { kind: unknown }).kind === "string"
  );
};

function App() {
  const [pingResult, setPingResult] = useState<string>("…");
  const [device, setDevice] = useState<DeviceState>({ status: "idle" });
  const streamEnabled = device.status === "found";

  useKeyboardTuning();

  const refreshDevice = useCallback(async () => {
    setDevice({ status: "checking" });
    try {
      const info = await checkDevice();
      console.info("[RAIL] RTL-SDR detected:", info);
      setDevice({ status: "found", device: info });
    } catch (err) {
      if (isRailError(err) && err.kind === "DeviceNotFound") {
        console.warn("[RAIL] No RTL-SDR device found");
        setDevice({
          status: "missing",
          message: "No RTL-SDR device found",
        });
      } else {
        const message = isRailError(err)
          ? `${err.kind}: ${err.message ?? "unknown error"}`
          : String(err);
        console.error("[RAIL] check_device failed:", err);
        setDevice({ status: "missing", message });
      }
    }
  }, []);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const reply = await ping();
        if (!cancelled) {
          setPingResult(reply);
          console.info("[RAIL] ping →", reply);
        }
      } catch (err) {
        if (!cancelled) {
          console.error("[RAIL] ping failed:", err);
          setPingResult("error");
        }
      }

      if (!cancelled) {
        await refreshDevice();
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [refreshDevice]);

  // Backend emits `device-status: connected=false` when librtlsdr's async
  // reader dies mid-stream (typical cause: dongle yanked out). Flip to the
  // "missing" view so the Refresh button surfaces again, and tell the
  // backend to release its stale session so a reconnect can start cleanly.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void subscribeDeviceStatus((payload) => {
      if (payload.connected) return;
      console.warn("[RAIL] device disconnected mid-stream:", payload.error);
      stopStream().catch((err) => {
        console.warn("[RAIL] stopStream after disconnect failed:", err);
      });
      setDevice({
        status: "missing",
        message: payload.error ?? "RTL-SDR disconnected",
      });
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return (
    <main className="app">
      <MenuBar />
      <header className="app-header">
        <h1>RAIL</h1>
        <div className="app-status">
          <span>
            IPC <code>{pingResult}</code>
          </span>
          <span>
            RTL-SDR{" "}
            {device.status === "idle" && <code>idle</code>}
            {device.status === "checking" && <code>checking…</code>}
            {device.status === "found" && (
              <code>
                {device.device.name} (#{device.device.index})
              </code>
            )}
            {device.status === "missing" && (
              <>
                <code>{device.message}</code>
                <button
                  type="button"
                  className="device-refresh"
                  onClick={() => {
                    void refreshDevice();
                  }}
                >
                  Refresh
                </button>
              </>
            )}
          </span>
        </div>
      </header>
      <section className="control-panel">
        <FrequencyControl />
        <div className="control-panel-row">
          <GainControl />
          <PpmControl />
        </div>
      </section>
      <Waterfall enabled={streamEnabled} />
    </main>
  );
}

export default App;
