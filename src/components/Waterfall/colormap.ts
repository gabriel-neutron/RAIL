// Waterfall colormap. Stops, hue choice and the monotonic-luminance
// requirement are specified in docs/DSP.md §3.
// Returns a packed Uint8ClampedArray of length `size * 3` (RGB triplets).

type Stop = [number, number, number];

const STOPS: Stop[] = [
  [4, 3, 1], // cold tube (below noise floor)
  [46, 25, 4],
  [104, 59, 9],
  [168, 105, 18],
  [224, 152, 31],
  [255, 196, 84],
  [255, 246, 226], // bloom (peaks)
];

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

export const buildColormapLut = (size: number): Uint8ClampedArray => {
  const lut = new Uint8ClampedArray(size * 3);
  const segments = STOPS.length - 1;
  for (let i = 0; i < size; i += 1) {
    const t = size === 1 ? 0 : i / (size - 1);
    const scaled = t * segments;
    const idx = Math.min(segments - 1, Math.floor(scaled));
    const local = scaled - idx;
    const a = STOPS[idx];
    const b = STOPS[idx + 1];
    const offset = i * 3;
    lut[offset] = lerp(a[0], b[0], local);
    lut[offset + 1] = lerp(a[1], b[1], local);
    lut[offset + 2] = lerp(a[2], b[2], local);
  }
  return lut;
};
