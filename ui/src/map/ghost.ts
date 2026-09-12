/**
 * The pixels a drag carries with it (M84).
 *
 * A transform drag is previewed as an outline and written once, on release —
 * applying it on every pointer report would bump the document revision, and
 * the revision addresses every tile (see `preview_transform`). So for the
 * length of the drag the field on screen is the field *before* it: the outline
 * follows the pointer and what it is an outline of sits still, which for a
 * macro — a rectangle of captured field, where the edge and the thing are
 * plainly two different objects — reads as the drag having no effect at all.
 *
 * The object's pixels are already on screen, so the drag can carry them: a
 * copy of the frame under the selection, taken once when the drag begins and
 * drawn back through the drag's own transform. It is a *preview*, and the
 * view is a proxy (invariant 3) — a similarity transform of screen pixels is
 * not what the re-render will produce at a different latitude, and it is not
 * asked to be. What lands is the re-render.
 */

/** Where a selection is, in the terms the handles are given in. */
export interface Placement {
  /** The pivot, in device pixels. */
  pivot: { x: number; y: number };
  /** Its orientation, degrees clockwise. */
  rotationDeg: number;
  /** Its reach on the ground, metres. A scale drag is the ratio of two. */
  radiusM: number;
}

/**
 * The 2-D transform that carries `from` to `to`, as canvas's six numbers
 * (`a, b, c, d, e, f`).
 *
 * A similarity: turn by the change in orientation, scale by the change in
 * reach, and put the old pivot on the new one. Screen y runs down, so a
 * clockwise turn on the map is a positive angle here.
 *
 * A reach of zero — a selection with no extent, which a point-like object at a
 * far zoom can be — scales by one rather than by infinity or nothing.
 */
export function ghostMatrix(
  from: Placement,
  to: Placement,
): [number, number, number, number, number, number] {
  const scale = from.radiusM > 0 && to.radiusM > 0 ? to.radiusM / from.radiusM : 1;
  const theta = ((to.rotationDeg - from.rotationDeg) * Math.PI) / 180;
  const a = scale * Math.cos(theta);
  const b = scale * Math.sin(theta);
  const c = -b;
  const d = a;
  return [
    a,
    b,
    c,
    d,
    to.pivot.x - (a * from.pivot.x + c * from.pivot.y),
    to.pivot.y - (b * from.pivot.x + d * from.pivot.y),
  ];
}

/** Applies [`ghostMatrix`] to a point, which is what the tests measure. */
export function applyMatrix(
  m: readonly [number, number, number, number, number, number],
  point: { x: number; y: number },
): { x: number; y: number } {
  return {
    x: m[0] * point.x + m[2] * point.y + m[4],
    y: m[1] * point.x + m[3] * point.y + m[5],
  };
}

/**
 * The square of screen the selection covers, clamped to the canvas.
 *
 * Taken from the reach the handles already carry rather than from the outline:
 * the reach is defined as covering every member's own footprint, so a box
 * around it needs no path and no union. `null` when the selection is entirely
 * off screen, which is a drag with nothing to carry.
 */
export function ghostRegion(
  pivot: { x: number; y: number },
  radius: { rx: number; ry: number },
  canvas: { width: number; height: number },
  padPx = 2,
): { left: number; top: number; width: number; height: number } | null {
  const left = Math.floor(pivot.x - radius.rx - padPx);
  const top = Math.floor(pivot.y - radius.ry - padPx);
  const right = Math.ceil(pivot.x + radius.rx + padPx);
  const bottom = Math.ceil(pivot.y + radius.ry + padPx);
  const clipped = {
    left: Math.max(0, left),
    top: Math.max(0, top),
    right: Math.min(canvas.width, right),
    bottom: Math.min(canvas.height, bottom),
  };
  const width = clipped.right - clipped.left;
  const height = clipped.bottom - clipped.top;
  if (width <= 0 || height <= 0) return null;
  return { left: clipped.left, top: clipped.top, width, height };
}
