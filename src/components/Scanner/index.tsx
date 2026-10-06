import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { subscribeIpcEvent } from "../../ipc/events";
import {
  EVENT_SCAN_COMPLETE,
  EVENT_SCAN_STEP,
  EVENT_SCAN_STOPPED,
} from "../../ipc/generated/events";
import { useRadioStore } from "../../store/radio";
import { useScannerStore } from "../../store/scanner";
import BandActivity from "./BandActivity";

export const Scanner = () => {
  const streaming = useRadioStore((s) => s.streaming);
  const setFrequency = useRadioStore((s) => s.setFrequency);

  const scanning = useScannerStore((s) => s.scanning);
  const frequenciesHz = useScannerStore((s) => s.frequenciesHz);
  const results = useScannerStore((s) => s.results);
  const endScan = useScannerStore((s) => s.endScan);
  const scanConfig = useScannerStore((s) => s.scanConfig);
  const runScanSession = useScannerStore((s) => s.runScanSession);
  const cancelScanSession = useScannerStore((s) => s.cancelScanSession);

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
  const [thresholdSnrDb, setThresholdSnrDb] = useState(() =>
    String(scanConfig.thresholdSnrDb),
  );
  const [statusText, setStatusText] = useState("Idle");
  const [selectedIdx, setSelectedIdx] = useState(-1);

  // When a band-menu click pushes new config, sync the form fields.
  // Object identity is the signal: only setScanConfig replaces the
  // object, and it must keep allocating a fresh one. User typing lives
  // in local state, so this never fights with an edit in progress.
  const [prevScanConfig, setPrevScanConfig] = useState(scanConfig);
  if (scanConfig !== prevScanConfig) {
    setPrevScanConfig(scanConfig);
    setStartMhz((scanConfig.startHz / 1e6).toFixed(1));
    setStopMhz((scanConfig.stopHz / 1e6).toFixed(1));
    setStepKhz(String(Math.round(scanConfig.stepHz / 1e3)));
    setDwellMs(String(scanConfig.dwellMs));
    setThresholdSnrDb(String(scanConfig.thresholdSnrDb));
  }

  // Ref so event callbacks always see the current threshold (SNR dB).
  const thresholdRef = useRef(scanConfig.thresholdSnrDb);
  useEffect(() => {
    const v = parseFloat(thresholdSnrDb);
    thresholdRef.current = Number.isFinite(v) ? v : 10;
  }, [thresholdSnrDb]);

  const threshold = useMemo(() => {
    const v = parseFloat(thresholdSnrDb);
    return Number.isFinite(v) ? v : 10;
  }, [thresholdSnrDb]);

  // Signals whose SNR is above the threshold — navigation targets.
  const detectedSignals = useMemo(
    () => results.filter((r) => (r.signalAvgDb - r.noiseFloorDb) > threshold),
    [results, threshold],
  );

  const selectedFrequencyHz =
    selectedIdx >= 0 && selectedIdx < detectedSignals.length
      ? detectedSignals[selectedIdx].frequencyHz
      : undefined;

  // Subscribe to all scanner events.
  useEffect(() => {
    let unlistenStep: (() => void) | undefined;
    let unlistenComplete: (() => void) | undefined;
    let unlistenStopped: (() => void) | undefined;
    let cancelled = false;

    // Mirror every hardware retune the scanner performs into the radio
    // store for display only. FrequencyAxis, FilterBandMarker, and
    // FrequencyControl read from that store. Going through setFrequency
    // would schedule a second retune of a frequency the scanner has
    // already tuned, landing inside its settle window and re-locking the
    // PLL under the measurement it is taking.
    void subscribeIpcEvent(EVENT_SCAN_STEP, (payload) => {
      useRadioStore.getState().syncFrequencyFromBackend(payload.frequencyHz);
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenStep = fn;
    });

    const autoSelect = () => {
      const { results: r } = useScannerStore.getState();
      const sigs = r.filter((x) => (x.signalAvgDb - x.noiseFloorDb) > thresholdRef.current);
      if (sigs.length > 0) {
        setSelectedIdx(0);
        setFrequency(sigs[0].frequencyHz);
      }
    };

    void subscribeIpcEvent(EVENT_SCAN_COMPLETE, () => {
      endScan();
      setStatusText("Done");
      autoSelect();
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenComplete = fn;
    });

    void subscribeIpcEvent(EVENT_SCAN_STOPPED, (payload) => {
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
    setStatusText("Starting…");
    const outcome = await runScanSession({
      startHz,
      stopHz,
      stepHz,
      dwellMs: dwell,
      squelchSnrDb: null,
    });
    setStatusText(outcome.ok ? "Scanning…" : `Error: ${outcome.message}`);
  }, [startMhz, stopMhz, stepKhz, dwellMs, runScanSession]);

  const handleStop = useCallback(async () => {
    await cancelScanSession();
    setStatusText("Stopped");
  }, [cancelScanSession]);

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
        <div className="scanner-field-row">
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
        </div>

        <div className="scanner-field-row">
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
        </div>

        <div className="scanner-field-row">
          <span className="scanner-label">Min SNR</span>
          <input
            type="number"
            className="scanner-input"
            value={thresholdSnrDb}
            onChange={(e) => setThresholdSnrDb(e.target.value)}
            step="1"
            min="0"
            aria-label="Minimum SNR threshold in dB"
          />
          <span className="scanner-unit">dB</span>
        </div>
      </div>

      {frequenciesHz.length > 0 && (
        <BandActivity
          frequenciesHz={frequenciesHz}
          results={results}
          threshold={threshold}
          selectedFrequencyHz={selectedFrequencyHz}
          onTune={handleTune}
        />
      )}

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
