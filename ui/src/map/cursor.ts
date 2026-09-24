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
import { mapColour } from "../settings/themes";

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
  /**
   * The pointer is over the active image layer's picture, which the hand
   * moves rather than pans (M36).
   *
   * Only the active layer's, for the reason `imageUnder` gives: a project
   * with several charts stacked would otherwise move whichever happened to
   * be on top, with no way to say which was meant.
   */
  onImage: boolean;
  /**
   * The image alignment mode is armed (Task 7): every click is a control
   * point's picture or map half, never a tool's. Crosshair, like a pick —
   * both are a click aimed at an exact point rather than at an object or a
   * stroke — but its own row, or a later change to the pick's cursor would
   * silently change this one too for a reason that has nothing to do with it.
   */
  aligning: boolean;
  /**
   * The tool in hand cannot work on the layer in hand (spec.md 6.1, M51).
   *
   * The backend has always refused these, but on release — after the stroke
   * had been drawn and previewed. The cursor says it on hover instead.
   */
  forbidden: boolean;
}

/**
 * A paint bucket, tipped toward the click. The hotspot is the lip of the
 * bucket, where the paint leaves it.
 */
const BUCKET_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">
<path d="M4.5 11.5 12 4l7.5 7.5-7.5 7.5z" fill="#c9e4ca" stroke="#2b3a46" stroke-width="1.5" stroke-linejoin="round"/>
<path d="M12 4v4" stroke="#2b3a46" stroke-width="1.5" stroke-linecap="round"/>
<path d="M19.5 13.5c1.5 2.2 2 3.4 2 4.3a2 2 0 0 1-4 0c0-.9.5-2.1 2-4.3z" fill="#87bba2" stroke="#2b3a46" stroke-width="1.2"/>
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
<path d="M12 2v6M12 16v6M2 12h6M16 12h6" stroke="#364958" stroke-width="3.4" stroke-linecap="round"/>
<path d="M12 2v6M12 16v6M2 12h6M16 12h6" stroke="#ffffff" stroke-width="1.6" stroke-linecap="round"/>
<path d="M8 8l8 8M16 8l-8 8" stroke="#364958" stroke-width="4" stroke-linecap="round"/>
<path d="M8 8l8 8M16 8l-8 8" stroke="#ff8fa0" stroke-width="2.2" stroke-linecap="round"/>
</svg>`;

/** The refusal as a CSS cursor, with the platform's own to fall back on. */
export const FORBIDDEN_CURSOR = `url("data:image/svg+xml;utf8,${encodeURIComponent(FORBIDDEN_SVG)}") 12 12, not-allowed`;

let bucketKey = "";
let bucketCursor = BUCKET_CURSOR;
function themedBucket(): string {
  const selection = mapColour("selection"), source = mapColour("source"), ink = mapColour("ink");
  const key = selection + source + ink;
  if (key !== bucketKey) {
    bucketKey = key;
    bucketCursor = BUCKET_CURSOR.replaceAll(encodeURIComponent("#c9e4ca"), encodeURIComponent(selection))
      .replaceAll(encodeURIComponent("#87bba2"), encodeURIComponent(source))
      .replaceAll(encodeURIComponent("#2b3a46"), encodeURIComponent(ink));
  }
  return bucketCursor;
}

/**
 * Which of the map's own modes claims a pointer-down ahead of every tool,
 * and ahead of each other in this order: a pick, then a running capture,
 * then the alignment mode.
 *
 * `cursorFor` below and `MapView`'s own pointer dispatch both call this, so
 * the cursor shown and the click actually handled cannot enforce a
 * different order from one another. That is not a hypothetical: a running
 * capture and an armed alignment session both being possible at once let
 * `cursorFor` promise the capture would win (`recording` checked before
 * `aligning`) while the dispatch checked alignment first and a click that
 * should have dragged the capture's region instead placed a control point
 * — silent, and `cursor.test.ts`'s own passing assertion about the cursor
 * string gave no sign anything was wrong (Task 7 review finding). A test
 * against this function's *return value*, not the cursor, is what pins the
 * dispatch order; a test of `cursorFor` alone cannot.
 */
export type PointerPriority = "picking" | "recording" | "aligning" | "tool";

export function pointerPriorityFor(context: {
  picking: boolean;
  recording: boolean;
  aligning: boolean;
}): PointerPriority {
  if (context.picking) return "picking";
  if (context.recording) return "recording";
  if (context.aligning) return "aligning";
  return "tool";
}

/** The CSS `cursor` value for a context. */
export function cursorFor(context: CursorContext): string {
  const priority = pointerPriorityFor(context);
  // A pick takes the click ahead of every tool, so its cursor comes first.
  if (priority === "picking") return "crosshair";
  // The eyedropper's magnifier is drawn on the overlay at the pointer; a
  // system cursor on top of it would hide the plus that marks the sample.
  if (context.eyedropper) return "none";
  // While a capture records, the region is dragged with the selection cursor
  // (spec.md 8.7): nothing else on the map does anything.
  if (priority === "recording") return "default";
  // Alignment takes every click, whatever the tool in hand (Task 7).
  if (priority === "aligning") return "crosshair";
  // The tool will not work on this layer (M51). Before the tools, which is
  // exactly where the refusal sits in the click.
  if (context.forbidden) return FORBIDDEN_CURSOR;
  if (context.insideRegion) return themedBucket();
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
