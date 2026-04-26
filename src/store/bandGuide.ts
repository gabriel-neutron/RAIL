import { create } from "zustand";

import { type BandCategory, type BandRegion } from "../data/bands";

export type BandGuideState = {
  visible: boolean;
  activeCategories: Set<BandCategory>;
  region: BandRegion;
  toggleVisible: () => void;
  toggleCategory: (category: BandCategory) => void;
  setRegion: (region: BandRegion) => void;
};

const ALL_CATEGORIES: BandCategory[] = [
  "broadcast",
  "aviation",
  "maritime",
  "amateur",
  "utility",
  "weather",
  "ism",
];

export const useBandGuideStore = create<BandGuideState>()((set) => ({
  visible: true,
  activeCategories: new Set(ALL_CATEGORIES),
  region: "global",
  toggleVisible: () => set((s) => ({ visible: !s.visible })),
  toggleCategory: (category) =>
    set((s) => {
      const next = new Set(s.activeCategories);
      if (next.has(category)) {
        next.delete(category);
      } else {
        next.add(category);
      }
      return { activeCategories: next };
    }),
  setRegion: (region) => set({ region }),
}));
