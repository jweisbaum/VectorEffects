/**
 * The pointer's cursor over the map, as one rule (M24).
 *
 * Before this the canvas had two cursors — `grab` everywhere, `crosshair` for
 * the brush and a pick — so every other tool showed a hand, the select tool
 * included, and nothing said a click inside a selected region would fill it.
 * The rule lives here, pure, so it can be read as a table and tested as one;
 * the map applies it on tool changes and on pointer moves.
 *
 * Every custom cursor is an inline SVG data URI: no file is fetched and
 * nothing leaves the bundle (invariant 5).
 */

import { CAPTURE, HAND, INSERT, MEASURE, SELECT, type ActiveTool } from "./tools";

/** What decides the cursor, beyond the tool in hand. */
export interface CursorContext {
  tool: ActiveTool;
  /** The eyedropper is armed: the overlay draws the magnifier, the cursor hides. */
  eyedropper: boolean;
  /** A position is waiting for a click — the inspector's or the tool's own. */
  picking: boolean;
  /** The pointer is inside a selected region with a tool that would fill it. */
  insideRegion: boolean;
  /** The hand is down and the map is being dragged. */
  panning: boolean;
  /** A macro capture is running: the region is dragged with the selection cursor. */
  recording: boolean;
}

/**
 * A paint bucket, tipped toward the click. The hotspot is the lip of the
 * bucket, where the paint leaves it.
 */
const BUCKET_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">
<path d="M4.5 11.5 12 4l7.5 7.5-7.5 7.5z" fill="#ffd666" stroke="#141c2c" stroke-width="1.5" stroke-linejoin="round"/>
<path d="M12 4v4" stroke="#141c2c" stroke-width="1.5" stroke-linecap="round"/>
<path d="M19.5 13.5c1.5 2.2 2 3.4 2 4.3a2 2 0 0 1-4 0c0-.9.5-2.1 2-4.3z" fill="#6fd9ff" stroke="#141c2c" stroke-width="1.2"/>
</svg>`;

/** The bucket as a CSS cursor, with a crosshair to fall back on. */
export const BUCKET_CURSOR = `url("data:image/svg+xml;utf8,${encodeURIComponent(BUCKET_SVG)}") 4 20, crosshair`;

/** The CSS `cursor` value for a context. */
export function cursorFor(context: CursorContext): string {
  // A pick takes the click ahead of every tool, so its cursor comes first.
  if (context.picking) return "crosshair";
  // The eyedropper's magnifier is drawn on the overlay at the pointer; a
  // system cursor on top of it would hide the plus that marks the sample.
  if (context.eyedropper) return "none";
  // While a capture records, the region is dragged with the selection cursor
  // (spec.md 8.7): nothing else on the map does anything.
  if (context.recording) return "default";
  if (context.insideRegion) return BUCKET_CURSOR;
  switch (context.tool) {
    case HAND:
      return context.panning ? "grabbing" : "grab";
    case SELECT:
    case CAPTURE:
    case INSERT:
      return "default";
    case MEASURE:
      return "crosshair";
    default:
      // Every tool that draws — the brush and its family, the shape fill, the
      // curve, the modifiers — aims a click, and the crosshair's centre is
      // the click. Where the tool has a hover indicator the overlay draws the
      // footprint around it.
      return "crosshair";
  }
}
