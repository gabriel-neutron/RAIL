import { useEffect, useRef } from "react";

import { setGain } from "../../ipc/commands";
import { useRadioStore } from "../../store/radio";
import HoverSlider from "./HoverSlider";

const SQUELCH_MIN_DBFS = -100;
const SQUELCH_MAX_DBFS = 0;
const SQUELCH_DEFAULT_DBFS = -60;

/// Volume level in [0..1] we restore to when the user unmutes from zero.
const UNMUTE_FALLBACK_VOLUME = 0.5;

const VolumeIcon = ({
  muted,
  level,
}: {
  muted: boolean;
  level: number;
}) => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="currentColor"
    stroke="currentColor"
    strokeWidth="0"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={{ transform: muted ? "scale(0.92)" : "scale(1)" }}
  >
    <path d="M11 5L6 9H2v6h4l5 4V5z" />
    {muted || level === 0 ? (
      <>
        <line x1="23" y1="9" x2="17" y2="15" strokeWidth="2" />
        <line x1="17" y1="9" x2="23" y2="15" strokeWidth="2" />
      </>
    ) : (
      <>
        {level >= 0.3 && (
          <path
            d="M15.54 8.46a5 5 0 0 1 0 7.07"
            fill="none"
            strokeWidth="2"
          />
        )}
        {level >= 0.7 && (
          <path
            d="M19.07 4.93a10 10 0 0 1 0 14.14"
            fill="none"
            strokeWidth="2"
          />
        )}
      </>
    )}
  </svg>
);

/// Signal bars with a dashed threshold line cutting through them when
/// the squelch gate is active. The dashed line reads as "gate at this
/// level" — audio below the line is silenced.
const SquelchIcon = ({ enabled }: { enabled: boolean }) => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <line x1="5" y1="20" x2="5" y2="16" />
    <line x1="10" y1="20" x2="10" y2="13" />
    <line x1="15" y1="20" x2="15" y2="9" />
    <line x1="20" y1="20" x2="20" y2="5" />
    {enabled && (
      <line
        x1="3"
        y1="14"
        x2="22"
        y2="14"
        strokeDasharray="2 2"
        strokeWidth="1.5"
      />
    )}
  </svg>
);

/// Antenna tower with concentric reception arcs. "A" badge in the
/// corner indicates automatic gain; plain tower means manual.
const GainIcon = ({ auto }: { auto: boolean }) => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <line x1="12" y1="21" x2="12" y2="12" />
    <path d="M7 13 Q12 8 17 13" />
    <path d="M4 15 Q12 5 20 15" />
    {auto && (
      <text
        x="19"
        y="23"
        fontSize="8"
        fontWeight="700"
        fill="currentColor"
        stroke="none"
      >
        A
      </text>
    )}
  </svg>
);

const formatGainTenths = (t: number) => `${(t / 10).toFixed(1)} dB`;

export const AudioControls = () => {
  const streaming = useRadioStore((s) => s.streaming);
  const volume = useRadioStore((s) => s.volume);
  const muted = useRadioStore((s) => s.muted);
  const squelchDbfs = useRadioStore((s) => s.squelchDbfs);
  const autoGain = useRadioStore((s) => s.autoGain);
  const gainTenths = useRadioStore((s) => s.gainTenthsDb);
  const gains = useRadioStore((s) => s.availableGainsTenthsDb);

  const setVolume = useRadioStore((s) => s.setVolume);
  const setMuted = useRadioStore((s) => s.setMuted);
  const setSquelchDbfs = useRadioStore((s) => s.setSquelchDbfs);
  const setAutoGain = useRadioStore((s) => s.setAutoGain);
  const setGainTenthsStore = useRadioStore((s) => s.setGainTenthsDb);

  // Remember the last "on" squelch threshold so toggling off → on
  // restores the user's previous pick instead of snapping to the
  // default.
  const lastSquelchRef = useRef<number>(squelchDbfs ?? SQUELCH_DEFAULT_DBFS);
  useEffect(() => {
    if (squelchDbfs !== null) lastSquelchRef.current = squelchDbfs;
  }, [squelchDbfs]);

  // Snap gain to a sensible default when the hardware-supplied list
  // doesn't contain our current pick (first connection, device swap).
  useEffect(() => {
    if (gains.length === 0) return;
    if (!gains.includes(gainTenths)) {
      setGainTenthsStore(gains[Math.floor(gains.length / 2)]);
    }
  }, [gains, gainTenths, setGainTenthsStore]);

  // --- Volume ---------------------------------------------------------
  const volumePct = Math.round(volume * 100);
  const effectiveVolumePct = muted ? 0 : volumePct;

  const handleVolumeChange = (next: number) => {
    const normalized = Math.max(0, Math.min(100, next)) / 100;
    setVolume(normalized);
    // Sliding above zero while muted is an implicit unmute — matches
    // how every OS volume control behaves.
    if (muted && normalized > 0) setMuted(false);
    // Sliding to zero implicitly mutes so the speaker icon updates
    // without a second click.
    if (!muted && normalized === 0) setMuted(true);
  };

  const toggleMute = () => {
    if (muted) {
      if (volume === 0) setVolume(UNMUTE_FALLBACK_VOLUME);
      setMuted(false);
    } else {
      setMuted(true);
    }
  };

  // --- Squelch --------------------------------------------------------
  const squelchEnabled = squelchDbfs !== null;
  const squelchValue = squelchDbfs ?? lastSquelchRef.current;

  const toggleSquelch = () => {
    setSquelchDbfs(squelchEnabled ? null : lastSquelchRef.current);
  };

  // --- Gain -----------------------------------------------------------
  const gainIdx = Math.max(0, gains.indexOf(gainTenths));
  const gainLabel = autoGain
    ? "auto"
    : gains.length > 0
      ? formatGainTenths(gains[gainIdx])
      : "—";

  const pushGainToHardware = (next: Parameters<typeof setGain>[0]) => {
    if (!streaming) return;
    setGain(next).catch((err) => {
      console.warn("[RAIL] set_gain failed:", err);
    });
  };

  const toggleAutoGain = () => {
    const next = !autoGain;
    setAutoGain(next);
    pushGainToHardware(
      next ? { auto: true } : { auto: false, tenthsDb: gainTenths },
    );
  };

  const handleGainChange = (idx: number) => {
    if (gains.length === 0) return;
    const clamped = Math.max(0, Math.min(gains.length - 1, idx));
    const tenths = gains[clamped];
    setGainTenthsStore(tenths);
    if (autoGain) return;
    pushGainToHardware({ auto: false, tenthsDb: tenths });
  };

  return (
    <div className="audio-controls">
      <HoverSlider
        icon={<VolumeIcon muted={muted} level={volume} />}
        iconLabel={muted ? "Unmute" : "Mute"}
        iconTooltip={muted ? "Volume · muted" : "Volume"}
        onIconClick={toggleMute}
        ariaLabel="Volume"
        value={volumePct}
        min={0}
        max={100}
        step={1}
        onChange={handleVolumeChange}
        valueLabel={muted ? "muted" : `${effectiveVolumePct}%`}
        collapsed={muted}
      />
      <HoverSlider
        icon={<GainIcon auto={autoGain} />}
        iconLabel={autoGain ? "Switch to manual gain" : "Switch to auto gain"}
        iconTooltip={autoGain ? "Gain · auto (click to set manually)" : "Gain"}
        onIconClick={toggleAutoGain}
        ariaLabel="Gain"
        value={gainIdx}
        min={0}
        max={Math.max(0, gains.length - 1)}
        step={1}
        onChange={handleGainChange}
        valueLabel={gainLabel}
        collapsed={autoGain || gains.length === 0}
      />
      <HoverSlider
        icon={<SquelchIcon enabled={squelchEnabled} />}
        iconLabel={squelchEnabled ? "Disable squelch" : "Enable squelch"}
        iconTooltip={
          squelchEnabled
            ? "Squelch · silences audio below threshold"
            : "Squelch · off"
        }
        onIconClick={toggleSquelch}
        ariaLabel="Squelch threshold"
        value={squelchValue}
        min={SQUELCH_MIN_DBFS}
        max={SQUELCH_MAX_DBFS}
        step={1}
        onChange={(v) => setSquelchDbfs(v)}
        valueLabel={squelchEnabled ? `${squelchValue.toFixed(0)} dBFS` : "off"}
        collapsed={!squelchEnabled}
      />
    </div>
  );
};

export default AudioControls;
