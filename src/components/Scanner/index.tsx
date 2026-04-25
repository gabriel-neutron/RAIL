import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { startScan, stopScan } from "../../ipc/commands";
import {
  subscribeScanComplete,
  subscribeScanStep,
  subscribeScanStopped,
} from "../../ipc/events";
import { useRadioStore } from "../../store/radio";
import { useScannerStore } from "../../store/scanner";
import BandActivity from "./BandActivity";

export const Scanner = () => {
  const streaming = useRadioStore((s) => s.streaming);
  const setFrequency = useRadioStore((s) => s.setFrequency);

  const scanning = useScannerStore((s) => s.scanning);
  const frequenciesHz = useScannerStore((s) => s.frequenciesHz);
  const results = useScannerStore((s) => s.results);
  const beginScan = useScannerStore((s) => s.beginScan);
  const endScan = useScannerStore((s) => s.endScan);
  const scanConfig = useScannerStore((s) => s.scanConfig);
  const scanConfigSeq = useScannerStore((s) => s.scanConfigSeq);

  const [startMhz, setStartMhz] = useState(() =>
    (scanConfig.startHz / 1e6).toFixed(1),
  );
  const [stopMhz, setStopMhz] = useState(() =>
    (scanConfig.stopHz / 1e6).toFixed(1),
  );
  const [stepKhz, setStepKhz] = useState(() =>
    String(Math.round(scanConfig.stepHz / 1e3)),
  );
  const [dwellMs, setDwellMs] = useState(() => String(scanConfig.dwellMs));
  const [thresholdDbfs, setThresholdDbfs] = useState(() =>
    String(scanConfig.thresholdDbfs),
  );
  const [statusText, setStatusText] = useState("Idle");
  const [selectedIdx, setSelectedIdx] = useState(-1);

  // When a band-menu click pushes new config, sync the form fields.
  // scanConfigSeq changes only on external setScanConfig calls, not on
  // user edits, so this never fights with in-progress typing.
  useEffect(() => {
    setStartMhz((scanConfig.startHz / 1e6).toFixed(1));
    setStopMhz((scanConfig.stopHz / 1e6).toFixed(1));
    setStepKhz(String(Math.round(scanConfig.stepHz / 1e3)));
    setDwellMs(String(scanConfig.dwellMs));
    setThresholdDbfs(String(scanConfig.thresholdDbfs));
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scanConfigSeq]);

  // Ref so event callbacks always see the current threshold.
  const thresholdRef = useRef(scanConfig.thresholdDbfs);
  useEffect(() => {
    const v = parseFloat(thresholdDbfs);
    thresholdRef.current = Number.isFinite(v) ? v : -70;
  }, [thresholdDbfs]);

  const threshold = useMemo(() => {
    const v = parseFloat(thresholdDbfs);
    return Number.isFinite(v) ? v : -70;
  }, [thresholdDbfs]);

  // Signals whose peak is above the threshold — navigation targets.
  const detectedSignals = useMemo(
    () => results.filter((r) => r.peakDbfs > threshold),
    [results, threshold],
  );

  const selectedFrequencyHz =
    selectedIdx >= 0 && selectedIdx < detectedSignals.length
      ? detectedSignals[selectedIdx].frequencyHz
      : undefined;

  const channelRef = useRef<Channel<ArrayBuffer> | null>(null);
  const freqsRef = useRef<number[]>([]);

  useEffect(() => {
    freqsRef.current = frequenciesHz;
  }, [frequenciesHz]);

  // Subscribe to all scanner events.
  useEffect(() => {
    let unlistenStep: (() => void) | undefined;
    let unlistenComplete: (() => void) | undefined;
    let unlistenStopped: (() => void) | undefined;
    let cancelled = false;

    // Keep the radio store in sync with every hardware retune the scanner
    // performs. FrequencyAxis, FilterBandMarker, and FrequencyControl all
    // read from that store, so they update automatically without any
    // direct coupling to the scanner.
    void subscribeScanStep((payload) => {
      useRadioStore.getState().setFrequency(payload.frequencyHz);
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenStep = fn;
    });

    const autoSelect = () => {
      const { results: r } = useScannerStore.getState();
      const sigs = r.filter((x) => x.peakDbfs > thresholdRef.current);
      if (sigs.length > 0) {
        setSelectedIdx(0);
        setFrequency(sigs[0].frequencyHz);
      }
    };

    void subscribeScanComplete(() => {
      endScan();
      setStatusText("Done");
      autoSelect();
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenComplete = fn;
    });

    void subscribeScanStopped((payload) => {
      endScan();
      setStatusText(`Stopped — ${(payload.frequencyHz / 1e6).toFixed(3)} MHz`);
      autoSelect();
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenStopped = fn;
    });

    return () => {
      cancelled = true;
      unlistenStep?.();
      unlistenComplete?.();
      unlistenStopped?.();
    };
  }, [endScan, setFrequency]);

  const handleStart = useCallback(async () => {
    const startHz = Math.round(parseFloat(startMhz) * 1e6);
    const stopHz = Math.round(parseFloat(stopMhz) * 1e6);
    const stepHz = Math.round(parseFloat(stepKhz) * 1e3);
    const dwell = Math.round(parseFloat(dwellMs));

    if (
      !Number.isFinite(startHz) ||
      !Number.isFinite(stopHz) ||
      !Number.isFinite(stepHz) ||
      !Number.isFinite(dwell)
    ) {
      setStatusText("Invalid parameters");
      return;
    }

    setSelectedIdx(-1);
    const channel = new Channel<ArrayBuffer>();
    channelRef.current = channel;

    channel.onmessage = (buffer: ArrayBuffer) => {
      const dbfs = new DataView(buffer).getFloat32(0, true);
      const freqs = freqsRef.current;
      const idx = useScannerStore.getState().results.length;
      if (idx < freqs.length) {
        useScannerStore
          .getState()
          .pushResult({ frequencyHz: freqs[idx], peakDbfs: dbfs });
      }
    };

    try {
      setStatusText("Starting…");
      const reply = await startScan(
        { startHz, stopHz, stepHz, dwellMs: dwell, squelchDbfs: null },
        channel,
      );
      beginScan(reply.frequenciesHz);
      setStatusText("Scanning…");
    } catch (err) {
      setStatusText(`Error: ${String(err)}`);
    }
  }, [startMhz, stopMhz, stepKhz, dwellMs, beginScan]);

  const handleStop = useCallback(async () => {
    try {
      await stopScan();
    } catch (err) {
      console.warn("[RAIL] stopScan failed:", err);
    }
    endScan();
    setStatusText("Stopped");
  }, [endScan]);

  const handleTune = useCallback(
    (frequencyHz: number) => {
      setFrequency(frequencyHz);
    },
    [setFrequency],
  );

  const handlePrev = useCallback(() => {
    if (detectedSignals.length === 0) return;
    const next =
      selectedIdx <= 0 ? detectedSignals.length - 1 : selectedIdx - 1;
    setSelectedIdx(next);
    setFrequency(detectedSignals[next].frequencyHz);
  }, [detectedSignals, selectedIdx, setFrequency]);

  const handleNext = useCallback(() => {
    if (detectedSignals.length === 0) return;
    const next =
      selectedIdx >= detectedSignals.length - 1 ? 0 : selectedIdx + 1;
    setSelectedIdx(next);
    setFrequency(detectedSignals[next].frequencyHz);
  }, [detectedSignals, selectedIdx, setFrequency]);

  const navLabel =
    detectedSignals.length === 0
      ? "—"
      : `${selectedIdx >= 0 ? selectedIdx + 1 : "—"}/${detectedSignals.length}`;

  return (
    <section className="scanner-panel" aria-label="Wideband scanner">
      <div className="scanner-header">Scanner</div>

      <div className="scanner-fields">
        <span className="scanner-label">Start</span>
        <input
          type="number"
          className="scanner-input"
          value={startMhz}
          onChange={(e) => setStartMhz(e.target.value)}
          disabled={scanning}
          step="0.1"
          min="0"
          aria-label="Scan start frequency in MHz"
        />
        <span className="scanner-unit">MHz</span>

        <span className="scanner-label">Stop</span>
        <input
          type="number"
          className="scanner-input"
          value={stopMhz}
          onChange={(e) => setStopMhz(e.target.value)}
          disabled={scanning}
          step="0.1"
          min="0"
          aria-label="Scan stop frequency in MHz"
        />
        <span className="scanner-unit">MHz</span>

        <span className="scanner-label">Step</span>
        <input
          type="number"
          className="scanner-input"
          value={stepKhz}
          onChange={(e) => setStepKhz(e.target.value)}
          disabled={scanning}
          step="100"
          min="1"
          aria-label="Scan step size in kHz"
        />
        <span className="scanner-unit">kHz</span>

        <span className="scanner-label">Dwell</span>
        <input
          type="number"
          className="scanner-input"
          value={dwellMs}
          onChange={(e) => setDwellMs(e.target.value)}
          disabled={scanning}
          step="50"
          min="50"
          aria-label="Dwell time per step in milliseconds"
        />
        <span className="scanner-unit">ms</span>

        <span className="scanner-label">Squelch</span>
        <input
          type="number"
          className="scanner-input"
          value={thresholdDbfs}
          onChange={(e) => setThresholdDbfs(e.target.value)}
          step="5"
          max="0"
          aria-label="Signal detection threshold in dBFS"
        />
        <span className="scanner-unit">dBFS</span>
      </div>

      <div className="scanner-separator" role="separator" />

      <BandActivity
        frequenciesHz={frequenciesHz}
        results={results}
        threshold={threshold}
        selectedFrequencyHz={selectedFrequencyHz}
        onTune={handleTune}
      />

      <div className="scanner-footer">
        <button
          type="button"
          className={
            scanning
              ? "scanner-btn scanner-btn-stop"
              : "scanner-btn scanner-btn-start"
          }
          disabled={!streaming}
          onClick={scanning ? () => void handleStop() : () => void handleStart()}
          title={!streaming ? "Start a stream first" : undefined}
        >
          {scanning ? "Stop" : "Start"}
        </button>

        <div className="scanner-nav">
          <button
            type="button"
            className="scanner-nav-btn"
            onClick={handlePrev}
            disabled={detectedSignals.length < 2}
            aria-label="Previous signal"
          >
            ‹
          </button>
          <span className="scanner-nav-label">{navLabel}</span>
          <button
            type="button"
            className="scanner-nav-btn"
            onClick={handleNext}
            disabled={detectedSignals.length < 2}
            aria-label="Next signal"
          >
            ›
          </button>
        </div>

        <span className="scanner-status">{statusText}</span>
      </div>
    </section>
  );
};

export default Scanner;
