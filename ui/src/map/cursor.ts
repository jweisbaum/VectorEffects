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
  /** The pointer is over the active image layer's picture, which a drag moves (M36). */
  onImage: boolean;
  /**
   * The tool in hand cannot work on the layer in hand (spec.md 6.1, M51).
   *
   * The backend has always refused these, but on release — after the stroke
   * had been drawn and previewed. The cursor says it on hover instead.
   */
  forbidden: boolean;
  /**
   * The picture grip under the pointer, if any (M50).
   *
   * A grip takes the drag ahead of every tool, so it takes the cursor too:
   * the crosshair of the tool in hand would promise a stroke where a drag
   * would resize a chart instead.
   */
  grip: "corner" | "edge" | "rotate" | null;
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

/**
 * A crosshair with a cross through it: the tool in hand will not work on the
 * layer in hand (M51).
 *
 * Drawn rather than taken from the platform's `not-allowed`, which is a
 * circle-and-slash that reads as "the whole map is dead" — this is about one
 * tool and one layer, and it keeps the crosshair so it still says where the
 * click would land if the layer were a different one. The hotspot stays at
 * the centre for the same reason.
 */
const FORBIDDEN_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">
<path d="M12 2v6M12 16v6M2 12h6M16 12h6" stroke="#141c2c" stroke-width="3.4" stroke-linecap="round"/>
<path d="M12 2v6M12 16v6M2 12h6M16 12h6" stroke="#ffffff" stroke-width="1.6" stroke-linecap="round"/>
<path d="M8 8l8 8M16 8l-8 8" stroke="#141c2c" stroke-width="4" stroke-linecap="round"/>
<path d="M8 8l8 8M16 8l-8 8" stroke="#ff8fa0" stroke-width="2.2" stroke-linecap="round"/>
</svg>`;

/** The refusal as a CSS cursor, with the platform's own to fall back on. */
export const FORBIDDEN_CURSOR = `url("data:image/svg+xml;utf8,${encodeURIComponent(FORBIDDEN_SVG)}") 12 12, not-allowed`;

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
  // A picture's grip takes the drag ahead of every tool (M50), so it takes
  // the cursor: `crosshair` here would promise a stroke that will not happen.
  // Ahead of the refusal below, since a grip works whatever the tool is —
  // that is what makes it a handle.
  if (context.grip === "corner") return "nwse-resize";
  if (context.grip === "edge") return "move";
  if (context.grip === "rotate") return "grab";
  // The tool will not work on this layer (M51). After the handles and before
  // the tools, which is exactly where the refusal sits in the click.
  if (context.forbidden) return FORBIDDEN_CURSOR;
  if (context.insideRegion) return BUCKET_CURSOR;
  switch (context.tool) {
    case HAND:
      if (context.panning) return "grabbing";
      // Over the picture the hand moves it rather than the map (M36), and
      // says so: a grab hand here would promise a pan that is not what a
      // drag does.
      return context.onImage ? "move" : "grab";
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
