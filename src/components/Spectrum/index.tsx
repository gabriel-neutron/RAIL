// Spectrum strip drawn above the waterfall from the same FFT frame.
//
// Just a canvas + styling. The parent (`Waterfall`) owns the rAF drain
// and imperatively draws to this canvas via the forwarded ref, which
// keeps the pair lockstep-aligned without a second IPC subscription.

import { forwardRef } from "react";

export type SpectrumProps = {
  className?: string;
};

export const Spectrum = forwardRef<HTMLCanvasElement, SpectrumProps>(
  ({ className }, ref) => {
    return (
      <canvas
        ref={ref}
        className={`spectrum-canvas${className ? ` ${className}` : ""}`}
        aria-label="Live spectrum"
      />
    );
  },
);

Spectrum.displayName = "Spectrum";

export default Spectrum;
