// Vertical dBFS bar driven by the `signal-level` event.
//
// Scale: -100 dBFS (bottom) … 0 dBFS (top). The fill tracks
// `currentDbfs`; a thin marker shows the backend's decaying peak-hold.
// See `docs/DSP.md` §2 and `src-tauri/src/ipc/commands.rs`.

import { useRadioStore } from "../../store/radio";

const FLOOR_DBFS = -100;
const CEIL_DBFS = 0;
const TICKS_DBFS = [0, -20, -40, -60, -80, -100] as const;

const clamp01 = (v: number): number => (v < 0 ? 0 : v > 1 ? 1 : v);

const toFraction = (dbfs: number): number => {
  if (!Number.isFinite(dbfs)) return 0;
  return clamp01((dbfs - FLOOR_DBFS) / (CEIL_DBFS - FLOOR_DBFS));
};

const formatDbfs = (dbfs: number | undefined): string => {
  if (dbfs === undefined || !Number.isFinite(dbfs)) return "— dBFS";
  return `${dbfs.toFixed(1)} dBFS`;
};

export const SignalMeter = () => {
  const signalLevel = useRadioStore((s) => s.signalLevel);
  const streaming = useRadioStore((s) => s.streaming);

  const currentFrac = toFraction(signalLevel?.currentDbfs ?? FLOOR_DBFS);
  const peakFrac = toFraction(signalLevel?.peakDbfs ?? FLOOR_DBFS);

  return (
    <section className="signal-meter" aria-label="Signal strength meter">
      <div className="signal-meter-header">Signal</div>
      <div className="signal-meter-body">
        <div className="signal-meter-ticks" aria-hidden="true">
          {TICKS_DBFS.map((db) => (
            <span
              key={db}
              className="signal-meter-tick"
              style={{ bottom: `${toFraction(db) * 100}%` }}
            >
              {db}
            </span>
          ))}
        </div>
        <div
          className="signal-meter-bar"
          role="meter"
          aria-valuemin={FLOOR_DBFS}
          aria-valuemax={CEIL_DBFS}
          aria-valuenow={signalLevel?.currentDbfs ?? FLOOR_DBFS}
        >
          <div
            className="signal-meter-fill"
            style={{ height: `${currentFrac * 100}%` }}
          />
          {signalLevel && streaming && (
            <div
              className="signal-meter-peak"
              style={{ bottom: `${peakFrac * 100}%` }}
            />
          )}
        </div>
      </div>
      <div className="signal-meter-readout">
        <div className="signal-meter-current">
          {formatDbfs(signalLevel?.currentDbfs)}
        </div>
        <div className="signal-meter-peak-text">
          peak {formatDbfs(signalLevel?.peakDbfs)}
        </div>
      </div>
    </section>
  );
};

export default SignalMeter;
