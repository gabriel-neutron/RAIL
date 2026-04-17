import { useEffect, useState } from "react";

import { setGain } from "../../ipc/commands";
import { useRadioStore } from "../../store/radio";

const formatDb = (tenths: number): string => `${(tenths / 10).toFixed(1)} dB`;

export const GainControl = () => {
  const streaming = useRadioStore((s) => s.streaming);
  const auto = useRadioStore((s) => s.autoGain);
  const setAuto = useRadioStore((s) => s.setAutoGain);
  const gainTenths = useRadioStore((s) => s.gainTenthsDb);
  const setGainStore = useRadioStore((s) => s.setGainTenthsDb);
  const gains = useRadioStore((s) => s.availableGainsTenthsDb);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (gains.length === 0) return;
    if (!gains.includes(gainTenths)) {
      setGainStore(gains[Math.floor(gains.length / 2)]);
    }
  }, [gains, gainTenths, setGainStore]);

  const disabled = !streaming;

  const handleAutoChange = async (next: boolean) => {
    setAuto(next);
    if (!streaming) return;
    try {
      await setGain(next ? { auto: true } : { auto: false, tenthsDb: gainTenths });
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  const handleSliderChange = async (idx: number) => {
    if (!gains.length) return;
    const tenths = gains[Math.max(0, Math.min(gains.length - 1, idx))];
    setGainStore(tenths);
    if (!streaming || auto) return;
    try {
      await setGain({ auto: false, tenthsDb: tenths });
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  const currentIdx = Math.max(0, gains.indexOf(gainTenths));

  return (
    <div className="gain-control">
      <label className="gain-control-auto">
        <input
          type="checkbox"
          checked={auto}
          disabled={disabled}
          onChange={(e) => {
            void handleAutoChange(e.target.checked);
          }}
        />
        <span>Auto gain</span>
      </label>
      {!auto && (
        <>
          <span className="gain-control-slider-label">Gain</span>
          <input
            type="range"
            className="gain-control-slider"
            min={0}
            max={Math.max(0, gains.length - 1)}
            step={1}
            value={currentIdx}
            disabled={disabled || gains.length === 0}
            onChange={(e) => {
              void handleSliderChange(Number(e.target.value));
            }}
          />
          <span className="gain-control-value">
            {gains.length > 0 ? formatDb(gains[currentIdx]) : "— dB"}
          </span>
        </>
      )}
      {error && <span className="gain-control-error">{error}</span>}
    </div>
  );
};

export default GainControl;
