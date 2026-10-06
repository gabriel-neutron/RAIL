import { useEffect, useState, type RefObject } from "react";

/// Counter bumped whenever `ref`'s element changes layout size.
///
/// Put it in a draw effect's dependency array so the effect re-runs on window
/// resize, panel resize, and the first layout after mount. Without it a
/// canvas's backing buffer stays sized for whatever width was current at first
/// paint, which leaves labels blurry afterward.
///
/// It returns a counter rather than the measured width on purpose: the width
/// is read inside the draw effect, from the canvas being drawn, so no measured
/// value has to survive a render.
export const useResizeTick = (ref: RefObject<Element | null>): number => {
  const [tick, setTick] = useState(0);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setTick((t) => t + 1));
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);

  return tick;
};
