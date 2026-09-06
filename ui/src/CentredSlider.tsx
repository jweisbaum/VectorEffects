/**
 * A slider for a signed amount whose zero means "do nothing" (M29): the
 * thumb starts in the middle, the two ends are named, and the value is read
 * out with its sign. What the intensity and the divergence are edited with.
 *
 * Local while the thumb is down and committed on release, the speed filter's
 * pattern: in the inspector a commit is a document write and an undo entry,
 * and one per tick would be one per pixel. `onInput` is for the option bar,
 * whose value is local state and whose preview wants every tick.
 */

import { useState } from "react";

import { positionOf, valueOf } from "./sliderMath";

/** The track's resolution: positions are integers on `-STEPS..STEPS`. */
const STEPS = 1000;

export default function CentredSlider({
  value,
  min,
  max,
  lowLabel,
  highLabel,
  reversed = false,
  format,
  title,
  onInput,
  onCommit,
}: {
  value: number;
  min: number;
  max: number;
  lowLabel: string;
  highLabel: string;
  /** The stored value's positive end is the *left* one. */
  reversed?: boolean;
  format: (value: number) => string;
  title?: string;
  /** Every tick, for a caller whose value is local. */
  onInput?: (value: number) => void;
  /** Once, on release. */
  onCommit: (value: number) => void;
}) {
  const [dragging, setDragging] = useState<number | null>(null);
  const shown = dragging ?? value;
  const sign = reversed ? -1 : 1;
  const position = Math.round(positionOf(shown, min, max) * sign * STEPS);
  const read = (raw: string) => valueOf((Number(raw) / STEPS) * sign, min, max);
  const endDrag = () => {
    if (dragging === null) return;
    setDragging(null);
    if (dragging !== value) onCommit(dragging);
  };
  return (
    <span className="centred-slider" title={title}>
      <span className="end">{lowLabel}</span>
      <input
        type="range"
        min={-STEPS}
        max={STEPS}
        step={1}
        value={position}
        onChange={(e) => {
          const next = read(e.target.value);
          setDragging(next);
          onInput?.(next);
        }}
        onPointerUp={endDrag}
        onKeyUp={endDrag}
        onBlur={endDrag}
      />
      <span className="end">{highLabel}</span>
      <span className="readout">{format(shown)}</span>
    </span>
  );
}

/** A signed percentage, `+50%`, `−20%`, `0%`. */
export function signedPercent(value: number): string {
  const rounded = Math.round(value);
  if (rounded === 0) return "0%";
  return `${rounded > 0 ? "+" : "−"}${Math.abs(rounded)}%`;
}
