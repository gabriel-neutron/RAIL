import { useEffect, useState } from "react";
import {
  checkDevice,
  ping,
  type DeviceInfo,
  type RailError,
} from "./ipc/commands";
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

      setDevice({ status: "checking" });
      try {
        const info = await checkDevice();
        if (!cancelled) {
          console.info("[RAIL] RTL-SDR detected:", info);
          setDevice({ status: "found", device: info });
        }
      } catch (err) {
        if (cancelled) return;
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
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="container">
      <h1>RAIL — Radio Analysis and Intel Lab</h1>
      <section className="status">
        <p>
          IPC bridge: <code>{pingResult}</code>
        </p>
        <p>
          RTL-SDR:{" "}
          {device.status === "idle" && <code>idle</code>}
          {device.status === "checking" && <code>checking…</code>}
          {device.status === "found" && (
            <code>
              {device.device.name} (index {device.device.index})
            </code>
          )}
          {device.status === "missing" && <code>{device.message}</code>}
        </p>
      </section>
    </main>
  );
}

export default App;
