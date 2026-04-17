import { useEffect, useMemo, useState } from "react";

import { UNIT_SCALE, useRadioStore, type FreqUnit } from "../../store/radio";

const UNITS: FreqUnit[] = ["Hz", "kHz", "MHz"];

const STEP_MULTIPLIERS = [-100, -10, -1, 1, 10, 100] as const;

// Conservative R820T2 tuning range (docs/HARDWARE.md §1).
const MIN_FREQ_HZ = 500_000;
const MAX_FREQ_HZ = 1_750_000_000;

const clampFreq = (hz: number): number =>
  Math.max(MIN_FREQ_HZ, Math.min(MAX_FREQ_HZ, Math.round(hz)));

const formatUnitValue = (hz: number, unit: FreqUnit): string => {
  const scaled = hz / UNIT_SCALE[unit];
  if (unit === "Hz") return scaled.toFixed(0);
  if (unit === "kHz") return scaled.toFixed(3);
  return scaled.toFixed(6);
};

const formatStepLabel = (n: number): string => (n > 0 ? `+${n}` : `${n}`);

export const FrequencyControl = () => {
  const frequencyHz = useRadioStore((s) => s.frequencyHz);
  const setFrequency = useRadioStore((s) => s.setFrequency);
  const unit = useRadioStore((s) => s.freqUnit);
  const setUnit = useRadioStore((s) => s.setFreqUnit);

  const [draft, setDraft] = useState<string>(() =>
    formatUnitValue(frequencyHz, unit),
  );
  const [focused, setFocused] = useState<boolean>(false);

  const canonical = useMemo(
    () => formatUnitValue(frequencyHz, unit),
    [frequencyHz, unit],
  );

  useEffect(() => {
    if (!focused) {
      setDraft(canonical);
    }
  }, [canonical, focused]);

  const commitDraft = (raw: string) => {
    const parsed = Number.parseFloat(raw.replace(",", "."));
    if (!Number.isFinite(parsed)) {
      setDraft(canonical);
      return;
    }
    setFrequency(clampFreq(parsed * UNIT_SCALE[unit]));
  };

  const bump = (multiplier: number) => {
    setFrequency(clampFreq(frequencyHz + multiplier * UNIT_SCALE[unit]));
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitDraft((e.target as HTMLInputElement).value);
      (e.target as HTMLInputElement).blur();
      return;
    }
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    const magnitude = e.shiftKey ? 10 : 1;
    const sign = e.key === "ArrowUp" ? 1 : -1;
    bump(sign * magnitude);
  };

  return (
    <section className="frequency-control">
      <span className="frequency-control-label">Center</span>
      <div className="frequency-control-row">
        <span className="frequency-control-value">{canonical}</span>
        <div className="frequency-control-units" role="radiogroup" aria-label="Unit">
          {UNITS.map((u) => (
            <button
              key={u}
              type="button"
              role="radio"
              aria-checked={unit === u}
              className={unit === u ? "unit-btn unit-btn-active" : "unit-btn"}
              onClick={() => setUnit(u)}
            >
              {u}
            </button>
          ))}
        </div>
      </div>
      <input
        className="frequency-control-input"
        type="text"
        inputMode="decimal"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={(e) => {
          setFocused(false);
          commitDraft(e.target.value);
        }}
        onKeyDown={handleKeyDown}
        aria-label={`Center frequency in ${unit}`}
      />
      <div className="frequency-control-steps" aria-label={`Step in ${unit}`}>
        {STEP_MULTIPLIERS.map((m) => (
          <button
            key={m}
            type="button"
            className="step-btn"
            onClick={() => bump(m)}
          >
            {formatStepLabel(m)}
          </button>
        ))}
      </div>
    </section>
  );
};

export default FrequencyControl;
