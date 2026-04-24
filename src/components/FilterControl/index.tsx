import { useEffect, useMemo } from "react";

import { useRadioStore, type DemodMode } from "../../store/radio";

/// Bandwidth presets per mode (Hz). Chosen from `docs/DSP.md` §4–5:
/// - WBFM broadcast lives in ±100 kHz (200 kHz channel).
/// - NBFM voice fits in 12.5/25 kHz.
/// - AM voice is usually 6/8/10 kHz.
const PRESETS_BY_MODE: Record<DemodMode, number[]> = {
  FM: [12_500, 25_000, 150_000, 200_000],
  NFM: [12_500, 25_000],
  AM: [6_000, 8_000, 10_000],
  USB: [2_700],
  LSB: [2_700],
  CW: [500],
};

const formatBandwidth = (hz: number): string => {
  if (hz >= 1_000) return `${(hz / 1_000).toFixed(hz % 1_000 === 0 ? 0 : 1)} kHz`;
  return `${hz} Hz`;
};

export const FilterControl = () => {
  const mode = useRadioStore((s) => s.mode);
  const bandwidthHz = useRadioStore((s) => s.bandwidthHz);
  const setBandwidth = useRadioStore((s) => s.setBandwidth);

  const presets = useMemo(() => PRESETS_BY_MODE[mode] ?? [], [mode]);
  const disabled = false;

  // When the mode changes, snap to a preset so the slider never lands
  // on an out-of-range value (e.g. 200 kHz bandwidth while in AM mode).
  useEffect(() => {
    if (presets.length === 0) return;
    if (!presets.includes(bandwidthHz)) {
      // FM broadcast defaults to widest (200 kHz); all other modes default to narrowest.
      const fallback = mode === "FM" ? presets[presets.length - 1] : presets[0];
      setBandwidth(fallback);
    }
  }, [mode, presets, bandwidthHz, setBandwidth]);

  return (
    <div className="filter-control">
      <span className="filter-control-label">Bandwidth</span>
      <div className="filter-control-buttons">
        {presets.map((hz) => (
          <button
            key={hz}
            type="button"
            className={
              hz === bandwidthHz
                ? "filter-btn filter-btn-active"
                : "filter-btn"
            }
            disabled={disabled}
            onClick={() => setBandwidth(hz)}
          >
            {formatBandwidth(hz)}
          </button>
        ))}
      </div>
    </div>
  );
};

export default FilterControl;
