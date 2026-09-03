/**
 * Gesture previews that outlive the gesture that drew them.
 *
 * A committed object is not on screen yet: the commit is an IPC round trip and
 * a tile render, and the map keeps drawing the previous revision until the new
 * tiles arrive (spec.md 5.4). Clearing the overlay at pointer-up therefore
 * leaves a gap in which the object exists in the document and nothing shows it,
 * which reads as the paint blinking out. The preview is held across that gap.
 *
 * Held for *every* tool, not for the brush: what is held is a footprint and the
 * field it carries, and neither knows which tool produced it.
 */

import type { Footprint } from "./footprint";

/**
 * Everything needed to draw a gesture's preview without reading tool state.
 *
 * A held preview carries the values its gesture froze at creation (spec.md 6.1)
 * rather than reading them back: by the time it is dropped the option bar may
 * say something else entirely, and the object would then be previewed as a wind
 * it is not.
 */
export interface FieldPreview {
  /** The region the gesture paints. */
  footprint: Footprint;
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

/** A gesture's preview being held until its field appears. */
export interface HeldPreview extends FieldPreview, Settling {}

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
