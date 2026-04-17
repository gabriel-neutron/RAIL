import { useEffect } from "react";

import { UNIT_SCALE, useRadioStore } from "../store/radio";

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
 * Global keyboard shortcuts for frequency tuning.
 *
 * - `ArrowUp`   : +1 × selected unit
 * - `ArrowDown` : -1 × selected unit
 * - `Shift + Arrow` : ×10
 * - `Ctrl  + Arrow` : ×100
 *
 * Ignored while an input/textarea/select/contenteditable holds focus so
 * typing a name or a frequency stays uninterrupted.
 */
export const useKeyboardTuning = (): void => {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
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
