import { useRadioStore } from "../../store/radio";

const formatMhz = (hz: number): string => {
  const mhz = hz / 1_000_000;
  return `${mhz.toFixed(3)} MHz`;
};

export const FrequencyControl = () => {
  const frequencyHz = useRadioStore((s) => s.frequencyHz);
  return (
    <section className="frequency-control">
      <span className="frequency-control-label">Center</span>
      <span className="frequency-control-value">{formatMhz(frequencyHz)}</span>
    </section>
  );
};

export default FrequencyControl;
