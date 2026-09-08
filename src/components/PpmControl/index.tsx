import { useEffect, useState } from "react";

import { useRadioStore } from "../../store/radio";
import { useReplayStore } from "../../store/replay";

export const PpmControl = () => {
  const streaming = useRadioStore((s) => s.streaming);
  const ppm = useRadioStore((s) => s.ppm);
  const applyPpm = useRadioStore((s) => s.applyPpm);
  const replayActive = useReplayStore((s) => s.active);

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
    try {
      // Clamping and the streaming/replay guard live behind the seam;
      // the field only owns parsing and what it shows.
      await applyPpm(parsed);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
    setDraft(String(useRadioStore.getState().ppm));
  };

  return (
    <div className="ppm-control">
      <span className="ppm-control-label">PPM</span>
      <input
        className="ppm-control-input"
        type="text"
        inputMode="numeric"
        value={draft}
        disabled={!streaming || replayActive}
        title={replayActive ? "PPM is fixed by the replayed file" : undefined}
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
