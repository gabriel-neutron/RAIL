import { useRadioStore, type DemodMode } from "../../store/radio";

const ACTIVE_MODES: DemodMode[] = ["FM", "AM"];
const STUBBED_MODES: DemodMode[] = ["USB", "LSB", "CW"];

export const ModeSelector = () => {
  const mode = useRadioStore((s) => s.mode);
  const setMode = useRadioStore((s) => s.setMode);

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
            className={mode === m ? "mode-btn mode-btn-active" : "mode-btn"}
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
            title="Coming in V1.1"
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
