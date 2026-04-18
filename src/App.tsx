import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  checkDevice,
  ping,
  stopStream,
  type DeviceInfo,
  type RailError,
} from "./ipc/commands";
import { subscribeDeviceStatus, subscribeSignalLevel } from "./ipc/events";
import AudioControls from "./components/AudioControls";
import FilterControl from "./components/FilterControl";
import FrequencyControl from "./components/FrequencyControl";
import MenuBar from "./components/MenuBar";
import ModeSelector from "./components/ModeSelector";
import PpmControl from "./components/PpmControl";
import SignalMeter from "./components/SignalMeter";
import StatusPill from "./components/StatusPill";
import Waterfall from "./components/Waterfall";
import useAudio from "./hooks/useAudio";
import useKeyboardTuning from "./hooks/useKeyboardTuning";
import { useRadioStore } from "./store/radio";
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

/// Matches `AUDIO_RATE_HZ` in `src-tauri/src/dsp/demod/mod.rs`. The
/// start_stream reply reports this rate verbatim; keeping a constant
/// lets the AudioContext initialize before the first reply arrives.
const AUDIO_SAMPLE_RATE_HZ = 44_100;

const deviceLabel = (d: DeviceState): string => {
  switch (d.status) {
    case "idle":
      return "idle";
    case "checking":
      return "checking…";
    case "found":
      return `${d.device.name} (#${d.device.index})`;
    case "missing":
      return d.message;
  }
};

function App() {
  const [pingResult, setPingResult] = useState<string>("…");
  const [device, setDevice] = useState<DeviceState>({ status: "idle" });
  const streamEnabled = device.status === "found";

  useKeyboardTuning();

  const { enqueue, resume } = useAudio({
    enabled: streamEnabled,
    sampleRateHz: AUDIO_SAMPLE_RATE_HZ,
  });
  const enqueueRef = useRef(enqueue);
  useEffect(() => {
    enqueueRef.current = enqueue;
  }, [enqueue]);
  const handleAudio = useMemo(
    () => (frame: Float32Array) => enqueueRef.current(frame),
    [],
  );

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
      useRadioStore.getState().setSignalLevel(null);
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

  // Signal-level events feed the store (which the SignalMeter reads).
  // Reset the level whenever streaming stops so the meter doesn't keep
  // showing stale peaks from the last session.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void subscribeSignalLevel((payload) => {
      useRadioStore.getState().setSignalLevel({
        currentDbfs: payload.current,
        peakDbfs: payload.peak,
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

  useEffect(() => {
    if (!streamEnabled) {
      useRadioStore.getState().setSignalLevel(null);
    }
  }, [streamEnabled]);

  // Any click in the app satisfies the browser's "user gesture before
  // audio playback" requirement. We call `resume` even if the context
  // isn't suspended — it's a no-op in that case.
  const handlePointerDown = useCallback(() => {
    void resume();
  }, [resume]);

  return (
    <main className="app" onPointerDown={handlePointerDown}>
      <MenuBar />
      <header className="app-header">
        <h1>RAIL</h1>
        <div className="app-status">
          <span>
            IPC <code>{pingResult}</code>
          </span>
          <StatusPill
            status={device.status}
            label={deviceLabel(device)}
            onRefresh={() => {
              void refreshDevice();
            }}
          />
        </div>
      </header>
      <section className="control-panel">
        <FrequencyControl />
        <div className="control-panel-row">
          <ModeSelector />
          <FilterControl />
        </div>
        <div className="control-panel-row">
          <AudioControls />
          <PpmControl />
        </div>
      </section>
      <div className="app-body">
        <Waterfall enabled={streamEnabled} onAudio={handleAudio} />
        <SignalMeter />
      </div>
    </main>
  );
}

export default App;
