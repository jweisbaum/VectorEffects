/**
 * The selection box over the timeline's grid (spec.md 9.3).
 *
 * The hit test, out of the component: it is the part with corners in it, and
 * the part that has to work the same whichever way the drag was made.
 */

/** A box as it was dragged, from the press to wherever the pointer is now. */
export interface DragBox {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/** The same box with its corners in order, so a test can compare against it. */
export function normalised(box: DragBox): DragBox {
  return {
    x0: Math.min(box.x0, box.x1),
    y0: Math.min(box.y0, box.y1),
    x1: Math.max(box.x0, box.x1),
    y1: Math.max(box.y0, box.y1),
  };
}

/**
 * Whether the key at `step`, on a row whose middle sits at `top`, is caught.
 *
 * The row is treated as a band of `halfRow` either side of its middle rather
 * than as a line: a key is a diamond on a 22-pixel row, and a box that
 * crossed the row would otherwise have to pass through its exact centre to
 * take anything.
 *
 * Direction-agnostic by construction — the box is put in order first — because
 * a drag up and to the left is the same rectangle as one down and to the
 * right, and the one that failed would fail silently.
 */
export function boxTakes(
  box: DragBox,
  step: number,
  pxPerStep: number,
  top: number,
  halfRow: number,
): boolean {
  const { x0, y0, x1, y1 } = normalised(box);
  if (top + halfRow < y0 || top - halfRow > y1) return false;
  const x = (step + 0.5) * pxPerStep;
  return x >= x0 && x <= x1;
}
