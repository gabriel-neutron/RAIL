import { type BandCategory, type BandRegion } from "../../data/bands";
import { useBandGuideStore } from "../../store/bandGuide";

const CATEGORIES: BandCategory[] = [
  "broadcast",
  "aviation",
  "maritime",
  "amateur",
  "utility",
  "weather",
  "ism",
];

const CATEGORY_LABELS: Record<BandCategory, string> = {
  broadcast: "BC",
  aviation:  "AIR",
  maritime:  "MAR",
  amateur:   "HAM",
  utility:   "UTIL",
  weather:   "WX",
  ism:       "ISM",
};

const REGIONS: BandRegion[] = ["global", "NA", "EU"];

export const BandGuideControls = () => {
  const visible = useBandGuideStore((s) => s.visible);
  const activeCategories = useBandGuideStore((s) => s.activeCategories);
  const region = useBandGuideStore((s) => s.region);
  const toggleVisible = useBandGuideStore((s) => s.toggleVisible);
  const toggleCategory = useBandGuideStore((s) => s.toggleCategory);
  const setRegion = useBandGuideStore((s) => s.setRegion);

  return (
    <div className="band-guide-controls">
      <button
        className={`band-guide-toggle${visible ? " is-active" : ""}`}
        onClick={toggleVisible}
        title={visible ? "Hide band guide" : "Show band guide"}
        aria-label={visible ? "Hide band guide" : "Show band guide"}
      >
        {visible ? "◉" : "○"}
      </button>
      {visible && (
        <>
          <span className="band-guide-sep" />
          {REGIONS.map((r) => (
            <button
              key={r}
              className={`band-guide-pill${region === r ? " is-active" : ""}`}
              onClick={() => setRegion(r)}
            >
              {r === "global" ? "GBL" : r}
            </button>
          ))}
          <span className="band-guide-sep" />
          {CATEGORIES.map((cat) => {
            const active = activeCategories.has(cat);
            return (
              <button
                key={cat}
                className={`band-guide-pill${active ? ` is-active band-guide-pill--${cat}` : ""}`}
                onClick={() => toggleCategory(cat)}
                title={cat}
              >
                {CATEGORY_LABELS[cat]}
              </button>
            );
          })}
        </>
      )}
    </div>
  );
};

export default BandGuideControls;
