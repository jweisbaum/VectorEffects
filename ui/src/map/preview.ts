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

import type { PreviewKind } from "../generated/PreviewKind";
import type { Footprint } from "./footprint";
import type { OperatorPreview } from "./renderer";

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
export interface HeldPreview extends FieldPreview, Settling {
  /**
   * The live operation this gesture was showing, for a tool that operates on
   * the field rather than adding one (spec.md 6.2).
   *
   * Held across the same gap the rest of the preview is. Dropping it at
   * pointer-up would let the masked region fill back in for the round trip it
   * takes the commit to arrive, and then empty again — the paint blinking out,
   * in reverse.
   */
  operator?: OperatorPreview;
  /**
   * Whether the overlay draws nothing for it. An operator's held preview is
   * the map's own doing — the operation kept applied until its tiles land —
   * and a field drawn over it would be a colour for a wind that is not there.
   */
  silent?: boolean;
}

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

/** What the overlay draws for a gesture in progress (spec.md 6.1, 6.2). */
export interface OverlayPlan {
  /** What goes over the swept region. */
  sweep: "field" | "outline" | "none";
  /** Whether the stamp under the pointer is outlined. */
  nib: boolean;
}

/**
 * What the overlay draws, from how the tool previews and whether a gesture is
 * in progress.
 *
 * A tool that paints a field draws it: the swept region in the ramp's colour
 * with glyphs over it, which is what the user is aiming (spec.md 6.1).
 *
 * A tool that *operates* on the field draws nothing over its sweep. The map is
 * already showing what it does, through a screen mask — the covered region loses its
 * field, the cloned one gains the source's (spec.md 6.2, D37) — and an outline
 * over that is not just redundant but wrong: a footprint is a *union* of
 * stamps and a stroked path is not a union, so outlining a sweep traces every
 * stamp's own circle and leaves a chain of rings trailing the pointer, which
 * is neither what either tool does nor what it looks like.
 *
 * Which is why an operator keeps its nib through the drag. It is the only
 * thing it draws, and without it a drag where there is nothing to operate on
 * would show nothing at all.
 */
export function overlayPlan(preview: PreviewKind, drawing: boolean): OverlayPlan {
  if (preview !== "field") return { sweep: "none", nib: true };
  return drawing ? { sweep: "field", nib: false } : { sweep: "none", nib: true };
}
