// The one frequency formatter. Five call sites, three digit policies.
//
// The policies differ because the callers genuinely need them to: a tick
// label's resolution must follow tick SPACING (labels 0.1 MHz apart need one
// decimal whatever their magnitude), a bookmark line needs a fixed, stable
// width so a list of them aligns, and a standalone readout with neither
// constraint derives its digits from its own magnitude.
//
// Unit choice is shared by all three: MHz at or above 1 MHz, kHz at or above
// 1 kHz, whole Hz below that.

export type FormatHzOptions = {
  /// Spacing, in Hz, between the values being labelled. When given, the unit
  /// and the fractional-digit count are just enough to resolve adjacent
  /// labels, and the other options are ignored.
  stepHz?: number;
  /// Fixed fractional digits in the chosen unit, instead of deriving them
  /// from the magnitude. Ignored below 1 kHz, which is always whole Hz.
  digits?: number;
  /// Drop the fraction when the value is a whole number of its unit, so
  /// `6 kHz` does not read `6.0 kHz` next to `2.7 kHz`.
  trimWholeUnits?: boolean;
};

const magnitudeDigits = (hz: number): number => {
  if (hz >= 1_000_000) return hz >= 10_000_000 ? 1 : 2;
  return hz >= 100_000 ? 0 : hz >= 10_000 ? 1 : 2;
};

const formatWithStep = (hz: number, step: number): string => {
  if (step >= 1_000_000) {
    const digits = Math.max(0, Math.min(6, -Math.floor(Math.log10(step)) + 6));
    return `${(hz / 1_000_000).toFixed(digits)} MHz`;
  }
  if (step >= 1_000) {
    const digits = Math.max(0, Math.min(6, -Math.floor(Math.log10(step)) + 3));
    return `${(hz / 1_000).toFixed(digits)} kHz`;
  }
  const digits = Math.max(0, -Math.floor(Math.log10(step)));
  return `${hz.toFixed(digits)} Hz`;
};

/// Format `hz` as a short unit-suffixed string.
///
/// With `stepHz` the resolution follows the step; with `digits` it is fixed;
/// with neither it follows `hz`'s own magnitude. Below 1 kHz the value is
/// always whole Hz, in every mode.
export const formatHz = (hz: number, options?: FormatHzOptions): string => {
  const step = options?.stepHz;
  if (step !== undefined) return formatWithStep(hz, step);

  if (hz < 1_000) return `${Math.round(hz)} Hz`;

  const inMega = hz >= 1_000_000;
  const scaled = hz / (inMega ? 1_000_000 : 1_000);
  const unit = inMega ? "MHz" : "kHz";
  if (options?.trimWholeUnits === true && Number.isInteger(scaled)) {
    return `${scaled.toFixed(0)} ${unit}`;
  }
  return `${scaled.toFixed(options?.digits ?? magnitudeDigits(hz))} ${unit}`;
};
