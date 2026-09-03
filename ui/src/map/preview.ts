/**
 * Stroke previews that outlive the gesture that drew them.
 *
 * A committed stroke is not on screen yet: the commit is an IPC round trip and
 * a tile render, and the map keeps drawing the previous revision until the new
 * tiles arrive (spec.md 5.4). Clearing the overlay at pointer-up therefore
 * leaves a gap in which the stroke exists in the document and nothing shows it,
 * which reads as the paint blinking out. The preview is held across that gap.
 */

import type { BrushShape } from "../generated/BrushShape";
import type { StampSpace } from "../generated/StampSpace";

/**
 * Everything needed to draw a stroke preview without reading tool state.
 *
 * A held preview carries the values its stroke froze at creation (spec.md 6.1)
 * rather than reading them back: by the time it is dropped the toolbar may say
 * something else entirely, and the stroke would then be previewed as a wind it
 * is not.
 */
export interface StrokePreview {
  points: ReadonlyArray<readonly [number, number]>;
  /** Half the brush's size in kilometres, resolved at creation (spec.md 3.5). */
  radiusKm: number;
  shape: BrushShape;
  /** Whether the stamp is a shape on the ground or on the map (spec.md 3.5). */
  space: StampSpace;
  /** Fill colour, from the speed ramp. */
  paint: string;
  /** Speed in knots, for the glyphs. */
  knots: number;
  /** The azimuth-toward a stamp at this position carries. */
  azimuthAt: (lon: number, lat: number) => number;
}

/**
 * A preview waiting for the field to catch up with it.
 *
 * Both an edit's own preview and a drag's outline wait the same way, so the
 * rule for when to drop one lives here once.
 */
export interface Settling {
  /** The revision the commit produced, or null until it returns. */
  revision: number | null;
  /** When the edit was committed, on the `performance.now()` clock. */
  at: number;
}

/** A stroke preview being held until its field appears. */
export interface HeldPreview extends StrokePreview, Settling {}

/**
 * How long a preview is held if its field never arrives, in milliseconds.
 *
 * The backstop for a tile that fails or a revision that never renders. Without
 * it a single failed fetch would leave a stroke painted on the overlay for the
 * rest of the session.
 */
export const SETTLE_TIMEOUT_MS = 4000;

/**
 * Whether a held preview can be dropped.
 *
 * Both conditions matter. The revision says the document contains the stroke;
 * outstanding tiles say the map is still drawing the revision before it, so
 * dropping the preview then would show the gap it exists to cover.
 */
export function previewHasLanded(
  preview: Settling,
  revision: number,
  pendingTiles: number,
  now: number,
): boolean {
  if (now - preview.at >= SETTLE_TIMEOUT_MS) return true;
  return preview.revision !== null && revision >= preview.revision && pendingTiles === 0;
}
