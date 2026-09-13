import type { ShapeControls } from "../generated/ShapeControls";

export interface ShapePointHit { ring: number; point: number }

/** The same projected positions are used for drawing and picking dots. */
export function hitShapePoint(
  controls: ShapeControls,
  project: (point: [number, number]) => { x: number; y: number },
  pointer: { x: number; y: number },
  reach: number,
): ShapePointHit | null {
  let best = reach;
  let hit: ShapePointHit | null = null;
  controls.rings.forEach((ring, r) => ring.forEach((point, p) => {
    const at = project(point);
    const distance = Math.hypot(at.x - pointer.x, at.y - pointer.y);
    if (distance <= best) { best = distance; hit = { ring: r, point: p }; }
  }));
  return hit;
}

/** A local drag preview; the authoritative edit is committed once on release. */
export function movedShapePoint(controls: ShapeControls, hit: ShapePointHit, to: [number, number]): ShapeControls {
  return { ...controls, rings: controls.rings.map((ring, r) => r === hit.ring
    ? ring.map((p, i) => i === hit.point ? to : p) : ring) };
}
