import { useEffect } from "react";

import { UNIT_SCALE, useRadioStore } from "../store/radio";
import { useScannerStore } from "../store/scanner";

/// Predicate: is focus on something that legitimately eats ArrowUp/Down
/// (a text input, textarea, select, or contenteditable region)?
const isTypingTarget = (el: Element | null): boolean => {
  if (!el) return false;
  if (el instanceof HTMLInputElement) return true;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLSelectElement) return true;
  if (el instanceof HTMLElement && el.isContentEditable) return true;
  return false;
};

/**
 * Global keyboard shortcuts for frequency tuning and scanning.
 *
 * - `ArrowUp`        : +1 × selected unit
 * - `ArrowDown`      : -1 × selected unit
 * - `Shift + Arrow`  : ×10
 * - `Ctrl  + Arrow`  : ×100
 * - `Ctrl+Shift+S`   : quick scan ±10 MHz around current frequency
 *
 * Ignored while an input/textarea/select/contenteditable holds focus so
 * typing a name or a frequency stays uninterrupted.
 */
export const useKeyboardTuning = (): void => {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      // Ctrl+Shift+S — quick scan ±10 MHz.
      if (e.key === "S" && e.ctrlKey && e.shiftKey) {
        e.preventDefault();
        const { frequencyHz, streaming, classifierEnabled } = useRadioStore.getState();
        if (!streaming || !classifierEnabled) return;
        const RANGE_HZ = 10_000_000;
        const startHz = Math.max(500_000, frequencyHz - RANGE_HZ);
        const stopHz = frequencyHz + RANGE_HZ;
        const stepHz = 200_000;
        const scannerStore = useScannerStore.getState();
        scannerStore.setScanConfig({ startHz, stopHz, stepHz, dwellMs: 200, thresholdSnrDb: 10 });
        if (!scannerStore.visible) scannerStore.toggleVisible();
        void scannerStore
          .runScanSession({ startHz, stopHz, stepHz, dwellMs: 200, squelchSnrDb: null })
          .then((outcome) => {
            if (!outcome.ok) {
              console.warn("[RAIL] quick scan failed:", outcome.message);
            }
          });
        return;
      }

      if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
      if (isTypingTarget(document.activeElement)) return;
      e.preventDefault();

      const { frequencyHz, freqUnit, setFrequency } = useRadioStore.getState();
      const magnitude = e.ctrlKey ? 100 : e.shiftKey ? 10 : 1;
      const sign = e.key === "ArrowUp" ? 1 : -1;
      setFrequency(frequencyHz + sign * magnitude * UNIT_SCALE[freqUnit]);
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
};

export default useKeyboardTuning;
