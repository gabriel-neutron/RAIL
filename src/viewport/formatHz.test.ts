import { describe, expect, it } from "vitest";

import { formatHz } from "./formatHz";

// Characterisation tests, not aspiration: these pin the exact strings
// FrequencyAxis's formatTick and FilterBandMarker's formatBandwidth produced
// before they were merged here. Any diff is a visible label change and must
// fail the build.

describe("formatHz with a step", () => {
  it("reproduces the axis tick labels", () => {
    expect(formatHz(101_100_000, { stepHz: 1_000_000 })).toBe("101 MHz");
    expect(formatHz(101_100_000, { stepHz: 500_000 })).toBe("101100 kHz");
    expect(formatHz(101_100_000, { stepHz: 100_000 })).toBe("101100 kHz");
    expect(formatHz(433_920_000, { stepHz: 10_000 })).toBe("433920 kHz");
    expect(formatHz(7_100_000, { stepHz: 1_000 })).toBe("7100 kHz");
    expect(formatHz(7_100_000, { stepHz: 500 })).toBe("7100000 Hz");
    expect(formatHz(1_200, { stepHz: 100 })).toBe("1200 Hz");
    expect(formatHz(1_234.5, { stepHz: 10 })).toBe("1235 Hz");
    expect(formatHz(1_234.567, { stepHz: 0.5 })).toBe("1234.6 Hz");
  });

  it("resolves adjacent ticks one step apart", () => {
    const stepHz = 0.5;
    expect(formatHz(1_000, { stepHz })).not.toBe(formatHz(1_000.5, { stepHz }));
  });
});

describe("formatHz without a step", () => {
  it("reproduces the filter marker's bandwidth labels", () => {
    expect(formatHz(2_700)).toBe("2.70 kHz");
    expect(formatHz(6_000)).toBe("6.00 kHz");
    expect(formatHz(12_500)).toBe("12.5 kHz");
    expect(formatHz(200_000)).toBe("200 kHz");
    expect(formatHz(1_500_000)).toBe("1.50 MHz");
    expect(formatHz(12_000_000)).toBe("12.0 MHz");
    expect(formatHz(999)).toBe("999 Hz");
    expect(formatHz(150)).toBe("150 Hz");
  });

  it("switches unit exactly at the thresholds", () => {
    expect(formatHz(1_000)).toBe("1.00 kHz");
    expect(formatHz(999.4)).toBe("999 Hz");
    expect(formatHz(1_000_000)).toBe("1.00 MHz");
  });
});

// Characterisation of the two variants absorbed last: MenuBar's
// formatFrequency (three decimals, whatever the magnitude) and
// FilterControl's formatBandwidth (whole-kHz presets show no fraction). Every string below is what the old
// private copies produced for a value the UI actually passes them.
describe("formatHz with fixed digits", () => {
  it("reproduces MenuBar's bookmark labels", () => {
    expect(formatHz(100_500_000, { digits: 3 })).toBe("100.500 MHz");
    expect(formatHz(433_920_000, { digits: 3 })).toBe("433.920 MHz");
    expect(formatHz(1_000_000, { digits: 3 })).toBe("1.000 MHz");
    expect(formatHz(7_100, { digits: 3 })).toBe("7.100 kHz");
    expect(formatHz(999, { digits: 3 })).toBe("999 Hz");
  });

  it("keeps the digit count fixed where the magnitude rule would not", () => {
    expect(formatHz(88_100_000, { digits: 3 })).toBe("88.100 MHz");
    expect(formatHz(88_100_000)).toBe("88.1 MHz");
    expect(formatHz(1_500_000, { digits: 3 })).toBe("1.500 MHz");
    expect(formatHz(1_500_000)).toBe("1.50 MHz");
  });
});

describe("formatHz with whole units trimmed", () => {
  it("reproduces FilterControl's every preset label", () => {
    const asPreset = (hz: number) =>
      formatHz(hz, { digits: 1, trimWholeUnits: true });
    expect(asPreset(500)).toBe("500 Hz");
    expect(asPreset(2_700)).toBe("2.7 kHz");
    expect(asPreset(6_000)).toBe("6 kHz");
    expect(asPreset(8_000)).toBe("8 kHz");
    expect(asPreset(10_000)).toBe("10 kHz");
    expect(asPreset(12_500)).toBe("12.5 kHz");
    expect(asPreset(25_000)).toBe("25 kHz");
    expect(asPreset(150_000)).toBe("150 kHz");
    expect(asPreset(200_000)).toBe("200 kHz");
  });
});
