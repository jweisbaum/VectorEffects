/**
 * Whether the tool's hover indicator is drawn, as one rule (M27).
 *
 * The indicator is the footprint a click would produce (spec.md 6.1). It is
 * drawn only where a click *would* produce it: a click that does something
 * else — fills the selected region, places an option, samples the field — has
 * its own indication, and the footprint under it says the wrong thing. The
 * rule lives here, pure, beside `cursorFor`, so the two can be read together
 * and tested as tables.
 */

/** What decides it, beyond the tool in hand. */
export interface HoverContext {
  /** The tool has a hover indicator at all (spec.md 6.2). */
  hasIndicator: boolean;
  /** The overlay plan draws a nib for this tool's preview kind. */
  nib: boolean;
  /** A position is waiting for a click: the click places it, not a footprint. */
  picking: boolean;
  /**
   * The pointer is inside a selected region with a tool that would fill it.
   * The click makes the object from the region, and the cursor is already the
   * paint bucket: a footprint drawn as well said the click would paint a spot.
   */
  insideRegion: boolean;
}

/** Whether the overlay draws the hover indicator. */
export function showsHoverIndicator(context: HoverContext): boolean {
  if (!context.hasIndicator || !context.nib) return false;
  if (context.picking) return false;
  if (context.insideRegion) return false;
  return true;
}
