import { useEffect, useState } from "react";

import { setPpm as setPpmCommand } from "../../ipc/commands";
import { useRadioStore } from "../../store/radio";

const MIN_PPM = -200;
const MAX_PPM = 200;

const clamp = (n: number): number => Math.max(MIN_PPM, Math.min(MAX_PPM, n | 0));

export const PpmControl = () => {
  const streaming = useRadioStore((s) => s.streaming);
  const ppm = useRadioStore((s) => s.ppm);
  const setPpm = useRadioStore((s) => s.setPpm);

  const [draft, setDraft] = useState<string>(String(ppm));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(String(ppm));
  }, [ppm]);

  const apply = async (raw: string) => {
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) {
      setDraft(String(ppm));
      return;
    }
    const next = clamp(parsed);
    setPpm(next);
    setDraft(String(next));
    if (!streaming) return;
    try {
      await setPpmCommand(next);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div className="ppm-control">
      <span className="ppm-control-label">PPM</span>
      <input
        className="ppm-control-input"
        type="text"
        inputMode="numeric"
        value={draft}
        disabled={!streaming}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={(e) => {
          void apply(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void apply((e.target as HTMLInputElement).value);
            (e.target as HTMLInputElement).blur();
          }
        }}
        aria-label="Crystal PPM correction"
      />
      {error && <span className="ppm-control-error">{error}</span>}
    </div>
  );
};

export default PpmControl;
