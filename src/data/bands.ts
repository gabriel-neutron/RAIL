// Frequency band reference data for the in-UI band guide.
// All allocations are approximate and typical — not legal or regulatory advice.
// region="global" = internationally coordinated (ITU).
// region="NA"     = North American typical allocation.
// region="EU"     = European typical allocation.

export type BandCategory =
  | "broadcast"
  | "aviation"
  | "maritime"
  | "amateur"
  | "utility"
  | "weather"
  | "ism";

export type BandRegion = "global" | "NA" | "EU";

export type BandEntry = {
  minHz: number;
  maxHz: number;
  label: string;
  shortLabel: string;
  category: BandCategory;
  region: BandRegion;
  /** 1 = always label, 2 = label when space permits, 3 = color bar only in dense views */
  priority: number;
};

export const BAND_ENTRIES: BandEntry[] = [
  // ---- Broadcast ----
  { minHz: 153_000,       maxHz: 279_000,       label: "AM LW",        shortLabel: "LW",    category: "broadcast", region: "EU",     priority: 2 },
  { minHz: 530_000,       maxHz: 1_710_000,     label: "AM MW",        shortLabel: "AM",    category: "broadcast", region: "NA",     priority: 1 },
  { minHz: 531_000,       maxHz: 1_602_000,     label: "AM MW",        shortLabel: "AM",    category: "broadcast", region: "EU",     priority: 1 },
  { minHz: 87_500_000,    maxHz: 108_000_000,   label: "FM Broadcast", shortLabel: "FM",    category: "broadcast", region: "global", priority: 1 },

  // ---- Aviation ----
  { minHz: 118_000_000,   maxHz: 137_000_000,   label: "Airband AM",   shortLabel: "AIR",   category: "aviation",  region: "global", priority: 1 },
  { minHz: 129_000_000,   maxHz: 137_000_000,   label: "ACARS",        shortLabel: "ACRS",  category: "aviation",  region: "global", priority: 2 },
  { minHz: 978_000_000,   maxHz: 979_000_000,   label: "UAT 978",      shortLabel: "UAT",   category: "aviation",  region: "NA",     priority: 2 },
  { minHz: 1_089_000_000, maxHz: 1_091_000_000, label: "ADS-B 1090",   shortLabel: "ADSB",  category: "aviation",  region: "global", priority: 1 },

  // ---- Maritime ----
  { minHz: 156_000_000,   maxHz: 174_000_000,   label: "Marine VHF",   shortLabel: "MAR",   category: "maritime",  region: "global", priority: 1 },
  { minHz: 161_975_000,   maxHz: 162_025_000,   label: "AIS",          shortLabel: "AIS",   category: "maritime",  region: "global", priority: 2 },

  // ---- Amateur ----
  { minHz: 144_000_000,   maxHz: 148_000_000,   label: "2m Ham",       shortLabel: "2m",    category: "amateur",   region: "NA",     priority: 1 },
  { minHz: 144_000_000,   maxHz: 146_000_000,   label: "2m Ham",       shortLabel: "2m",    category: "amateur",   region: "EU",     priority: 1 },
  { minHz: 420_000_000,   maxHz: 450_000_000,   label: "70cm Ham",     shortLabel: "70cm",  category: "amateur",   region: "NA",     priority: 1 },
  { minHz: 430_000_000,   maxHz: 440_000_000,   label: "70cm Ham",     shortLabel: "70cm",  category: "amateur",   region: "EU",     priority: 1 },
  { minHz: 1_240_000_000, maxHz: 1_300_000_000, label: "23cm Ham",     shortLabel: "23cm",  category: "amateur",   region: "global", priority: 3 },

  // ---- Utility / LMR ----
  { minHz: 151_000_000,   maxHz: 159_000_000,   label: "Business LMR", shortLabel: "LMR",   category: "utility",   region: "NA",     priority: 3 },
  { minHz: 152_000_000,   maxHz: 157_000_000,   label: "Pager/POCSAG", shortLabel: "PAGE",  category: "utility",   region: "NA",     priority: 2 },
  { minHz: 159_000_000,   maxHz: 162_000_000,   label: "Railroad",     shortLabel: "RAIL",  category: "utility",   region: "NA",     priority: 2 },
  { minHz: 380_000_000,   maxHz: 400_000_000,   label: "TETRA",        shortLabel: "TETRA", category: "utility",   region: "EU",     priority: 2 },
  { minHz: 446_006_250,   maxHz: 446_093_750,   label: "PMR446",       shortLabel: "PMR",   category: "utility",   region: "EU",     priority: 1 },
  { minHz: 462_000_000,   maxHz: 467_000_000,   label: "FRS/GMRS",     shortLabel: "FRS",   category: "utility",   region: "NA",     priority: 1 },
  { minHz: 890_000_000,   maxHz: 960_000_000,   label: "GSM 900",      shortLabel: "GSM",   category: "utility",   region: "EU",     priority: 2 },

  // ---- Weather ----
  { minHz: 162_400_000,   maxHz: 162_550_000,   label: "NOAA WX",      shortLabel: "WX",    category: "weather",   region: "NA",     priority: 1 },

  // ---- ISM ----
  { minHz: 433_050_000,   maxHz: 434_790_000,   label: "ISM 433",      shortLabel: "ISM",   category: "ism",       region: "EU",     priority: 1 },
  { minHz: 902_000_000,   maxHz: 928_000_000,   label: "ISM 915",      shortLabel: "ISM",   category: "ism",       region: "NA",     priority: 1 },
];
