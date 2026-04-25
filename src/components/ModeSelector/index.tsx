import { useRadioStore, type DemodMode } from "../../store/radio";

const ACTIVE_MODES: DemodMode[] = ["FM", "NFM", "AM", "USB", "LSB", "CW"];
const STUBBED_MODES: DemodMode[] = [];

export const ModeSelector = () => {
  const mode = useRadioStore((s) => s.mode);
  const setMode = useRadioStore((s) => s.setMode);
  const classification = useRadioStore((s) => s.classification);

  const classForMode = (m: DemodMode): string => {
    const classes = ["mode-btn"];
    if (mode === m) classes.push("mode-btn-active");
    if (classification?.confirmed === m) classes.push("mode-btn-confirmed");
    else if (classification?.candidates.includes(m)) classes.push("mode-btn-suggested");
    return classes.join(" ");
  };

  return (
    <div className="mode-selector" role="radiogroup" aria-label="Demodulator mode">
      <span className="mode-selector-label">Mode</span>
      <div className="mode-selector-buttons">
        {ACTIVE_MODES.map((m) => (
          <button
            key={m}
            type="button"
            role="radio"
            aria-checked={mode === m}
            className={classForMode(m)}
            onClick={() => setMode(m)}
          >
            {m}
          </button>
        ))}
        {STUBBED_MODES.map((m) => (
          <button
            key={m}
            type="button"
            role="radio"
            aria-checked={false}
            aria-disabled
            className="mode-btn mode-btn-disabled"
            title="CW (Morse) — coming soon"
            disabled
          >
            {m}
          </button>
        ))}
      </div>
    </div>
  );
};

export default ModeSelector;
