// Icon-first slider that reveals its track on hover. Pattern lifted
// from the reference `VolumeControl` snippet in the Phase 3 UX pass:
// compact icon button + width-animated track + right-anchored value
// readout, with a short grace period before hiding so the user can
// ferry the cursor onto the track.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";

const HIDE_DELAY_MS = 200;

export type HoverSliderProps = {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (v: number) => void;
  icon: ReactNode;
  iconLabel: string;
  /// Optional native tooltip text for the icon button. Defaults to
  /// `iconLabel` if omitted. Useful when the icon alone isn't
  /// self-explanatory (e.g. squelch, gain).
  iconTooltip?: string;
  onIconClick?: () => void;
  ariaLabel: string;
  /// Text to render to the right of the track; when `collapsed` is
  /// true the caller can pass a distinct string (e.g. "off").
  valueLabel: string;
  /// When true the track stays hidden even on hover and the value
  /// label renders at reduced opacity. Use for muted/disabled states.
  collapsed?: boolean;
  /// When true the icon button is disabled, the track cannot open and
  /// the value label renders at reduced opacity. Use when the slider
  /// represents a hardware control that doesn't apply (e.g. gain
  /// during IQ replay).
  disabled?: boolean;
  trackWidthPx?: number;
};

export const HoverSlider = ({
  value,
  min,
  max,
  step = 1,
  onChange,
  icon,
  iconLabel,
  iconTooltip,
  onIconClick,
  ariaLabel,
  valueLabel,
  collapsed = false,
  disabled = false,
  trackWidthPx = 64,
}: HoverSliderProps) => {
  const [hovered, setHovered] = useState(false);
  const [dragging, setDragging] = useState(false);
  const trackRef = useRef<HTMLDivElement | null>(null);
  const hideTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fillPercent =
    max === min ? 0 : Math.max(0, Math.min(1, (value - min) / (max - min))) * 100;

  const setFromClientX = useCallback(
    (clientX: number) => {
      const el = trackRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0) return;
      const x = Math.max(0, Math.min(clientX - rect.left, rect.width));
      const raw = min + (x / rect.width) * (max - min);
      const snapped = Math.round(raw / step) * step;
      onChange(Math.max(min, Math.min(max, snapped)));
    },
    [min, max, step, onChange],
  );

  const handleTrackPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    setDragging(true);
    setFromClientX(e.clientX);
  };

  useEffect(() => {
    if (!dragging) return;
    const move = (e: PointerEvent) => setFromClientX(e.clientX);
    const up = () => setDragging(false);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [dragging, setFromClientX]);

  const clearHideTimer = () => {
    if (hideTimeoutRef.current) {
      clearTimeout(hideTimeoutRef.current);
      hideTimeoutRef.current = null;
    }
  };

  const handleMouseEnter = () => {
    clearHideTimer();
    setHovered(true);
  };

  const handleMouseLeave = () => {
    // Keep the track up while the user is actively scrubbing — they
    // may have slid the cursor past the track edge mid-drag.
    if (dragging) return;
    hideTimeoutRef.current = setTimeout(() => setHovered(false), HIDE_DELAY_MS);
  };

  useEffect(
    () => () => {
      clearHideTimer();
    },
    [],
  );

  const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const delta = step;
    if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
      e.preventDefault();
      onChange(Math.max(min, value - delta));
    } else if (e.key === "ArrowRight" || e.key === "ArrowUp") {
      e.preventDefault();
      onChange(Math.min(max, value + delta));
    } else if (e.key === "Home") {
      e.preventDefault();
      onChange(min);
    } else if (e.key === "End") {
      e.preventDefault();
      onChange(max);
    }
  };

  const show = (hovered || dragging) && !collapsed && !disabled;

  return (
    <div
      className="hover-slider"
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      aria-disabled={disabled || undefined}
    >
      <button
        type="button"
        className="hover-slider-btn"
        onClick={onIconClick}
        disabled={disabled}
        aria-label={iconLabel}
        title={iconTooltip ?? iconLabel}
      >
        {icon}
      </button>
      <div
        className="hover-slider-rail"
        style={{
          width: show ? `${trackWidthPx + 12}px` : "0px",
          opacity: show ? 1 : 0,
        }}
      >
        <div
          ref={trackRef}
          className="hover-slider-track"
          role="slider"
          aria-label={ariaLabel}
          aria-valuenow={value}
          aria-valuemin={min}
          aria-valuemax={max}
          tabIndex={0}
          onPointerDown={handleTrackPointerDown}
          onKeyDown={handleKeyDown}
          style={{ width: `${trackWidthPx}px` }}
        >
          <div className="hover-slider-track-bg" />
          <div
            className="hover-slider-track-fill"
            style={{ width: `${fillPercent}%` }}
          />
          <div
            className="hover-slider-thumb"
            style={{ left: `calc(${fillPercent}% - 5px)` }}
          />
        </div>
      </div>
      <span
        className="hover-slider-value"
        style={{ opacity: disabled || collapsed ? 0.5 : 1 }}
      >
        {valueLabel}
      </span>
    </div>
  );
};

export default HoverSlider;
