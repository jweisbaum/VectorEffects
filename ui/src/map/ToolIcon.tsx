/**
 * The palette's icons.
 *
 * Inline SVG, drawn here rather than pulled from an icon set: invariant 5 for-
 * bids any runtime network access, so a webfont or a CDN sprite is out, and a
 * bundled icon package would be a dependency carried for eleven glyphs. These
 * are eleven sets of paths.
 *
 * Everything is stroked in `currentColor` on a 24-unit grid, so a button's
 * colour — including the active state's — reaches the icon without the icon
 * knowing anything about the theme.
 *
 * The icon is decoration: the button's accessible name comes from its
 * `aria-label`, which is the tool's own label from the schema. Replacing a
 * word with a picture must not take the word away from anyone reading the page
 * with something other than their eyes.
 */

import type { ReactElement } from "react";

import { FILL, HAND, SELECT, type ActiveTool } from "./tools";

/** One icon: the geometry inside a `0 0 24 24` box. */
type Icon = readonly ReactElement[];

/**
 * A rounded stroke, which is every line in the set.
 *
 * Given as a component rather than repeated per path so the icons cannot
 * drift into different weights.
 */
function line(d: string, key: string, filled = false): ReactElement {
  return (
    <path
      key={key}
      d={d}
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  );
}

/**
 * The icons, one per tool.
 *
 * A `Record` over the tool union rather than a lookup with a fallback: adding a
 * tool to the catalogue then fails to compile until it has an icon, which is
 * the same guarantee the Rust side gets from matching on `ToolKind`
 * exhaustively. A fallback would have shipped a blank button instead.
 */
const ICONS: Record<ActiveTool, Icon> = {
  // The select tool: a dashed marquee, the shape every paint application
  // draws for "an area, not a thing".
  [SELECT]: [
    line("M4 7V5a1 1 0 0 1 1-1h2", "corner-nw"),
    line("M17 4h2a1 1 0 0 1 1 1v2", "corner-ne"),
    line("M20 17v2a1 1 0 0 1-1 1h-2", "corner-se"),
    line("M7 20H5a1 1 0 0 1-1-1v-2", "corner-sw"),
    line("M10 4h4M10 20h4M4 10v4M20 10v4", "dashes"),
  ],
  // The patch: a rectangle of captured field, dashed on the side it came from
  // and solid where it is now. Never in the palette — a patch is pasted, not
  // drawn — but the panels name every object by its tool, so it needs a mark.
  patch: [
    line("M4 8h9v9H4z", "pasted"),
    line("M8 4h9v9h-2M11 4v2", "taken"),
  ],
  // The macro: a stack of captured frames, the front one solid.
  macro: [
    line("M4 9h11v11H4z", "front"),
    line("M7 6h11v11h-3", "middle"),
    line("M10 3h11v11h-3", "back"),
  ],
  // The fill tool: a bucket tipped over what it fills.
  [FILL]: [
    line("M11 4 4 11a2 2 0 0 0 0 3l5 5a2 2 0 0 0 3 0l7-7z", "bucket"),
    line("M5.2 10.4h13.2", "rim"),
    line("M20 15c1.2 1.6 1.8 2.6 1.8 3.4A1.8 1.8 0 0 1 18.2 18.4c0-.8.6-1.8 1.8-3.4z", "drip"),
  ],
  // The hand that pans, with the thumb and three fingers a hand icon needs to
  // read as one at 18 pixels.
  [HAND]: [
    line(
      "M9 11V5.5a1.5 1.5 0 0 1 3 0V11m0-1.5a1.5 1.5 0 0 1 3 0V11m0-1a1.5 1.5 0 0 1 3 0v5.5a5 5 0 0 1-5 5h-1.6a5 5 0 0 1-3.9-1.9L6 15.5a1.6 1.6 0 0 1 2.4-2.1L9 14",
      "palm",
    ),
  ],

  // A brush held at an angle, with its ferrule and a loaded tip.
  brush: [
    line("M17.5 3.8 20.2 6.5 11 15.7 8.3 13z", "handle"),
    line("M8.3 13 11 15.7l-1.6 1.7a3.4 3.4 0 0 1-4.8-4.8z", "bristles", true),
  ],

  // Concentric rings about a centre: a stamp whose field is radial, which is
  // what its fill modes describe — a disc, a ring, or a ramp between them.
  // Deliberately not a circle with an arrow round it, which reads as "reload".
  circle: [
    line("M12 4.4a7.6 7.6 0 1 1 0 15.2 7.6 7.6 0 0 1 0-15.2z", "rim"),
    line("M12 8.6a3.4 3.4 0 1 1 0 6.8 3.4 3.4 0 0 1 0-6.8z", "inner"),
    line("M12 11.2a0.8 0.8 0 1 1 0 1.6 0.8 0.8 0 0 1 0-1.6z", "centre", true),
  ],

  // A flat polygon with the fill running across it. An outline with interior
  // edges reads as a solid — a box seen in three dimensions — where this tool
  // draws a shape on a map and floods it.
  shape_fill: [
    line("M12 3.8 20.2 9.8 17.1 19.4 6.9 19.4 3.8 9.8z", "outline"),
    line("M7.2 12.2h9.6M8.4 15.4h7.2", "fill"),
  ],

  // A frame with a hole in it: what a mask is. The hole is drawn as a second
  // ring rather than as a cut-out, since these icons are stroked and a
  // fill-rule hole would vanish.
  mask: [
    line("M3.6 5.2h16.8v13.6H3.6z", "frame"),
    line("M9 12a3 3 0 1 1 6 0 3 3 0 0 1-6 0z", "hole"),
  ],

  // A rubber stamp: the pad, the shaft and the handle above it.
  clone_stamp: [
    line("M5.5 20.4h13", "base"),
    line("M6.8 17.4h10.4v-2.1a2 2 0 0 0-2-2H8.8a2 2 0 0 0-2 2z", "pad"),
    line("M10 13.3V9.9M14 13.3V9.9", "shaft"),
    line("M8.6 9.9h6.8L14 4.9a1.4 1.4 0 0 0-1.3-1H11.3a1.4 1.4 0 0 0-1.3 1z", "handle"),
  ],

  // A Bézier between two nodes. The nodes are what a path tool is: points you
  // place and then pull. Ticks standing off the ends read as legs instead, so
  // the handles are left to the gesture, which draws them for real.
  curve: [
    line("M4.8 17.2C4.8 9.2 19.2 14.8 19.2 6.8", "curve"),
    line("M3.4 15.8h2.8v2.8H3.4z", "start", true),
    line("M17.8 5.4h2.8v2.8h-2.8z", "end", true),
  ],

  // --- The modifiers (spec.md 6.3) ---
  //
  // Each says what it does to a flow rather than what it looks like as an
  // object: they are all the same disc, so a disc would tell the user nothing.

  // Two vectors, one short and one long: the same flow, more of it or less.
  intensity: [
    line("M4.6 8.6h5.6m-2 -2 2 2-2 2", "short"),
    line("M4.6 15.4h13.2m-2.4-2.4 2.4 2.4-2.4 2.4", "long"),
  ],

  // Vectors leaving a centre. Divergence is the outward sign, and the tool
  // reaches convergence with a negative amount, as its label says.
  divergence: [
    line("M12 9.4V4.8m-1.8 1.8L12 4.8l1.8 1.8", "north"),
    line("M12 14.6v4.6m-1.8-1.8L12 19.2l1.8-1.8", "south"),
    line("M9.4 12H4.8m1.8-1.8L4.8 12l1.8 1.8", "west"),
    line("M14.6 12h4.6m-1.8-1.8L19.2 12l-1.8 1.8", "east"),
    line("M12 11.2a0.8 0.8 0 1 1 0 1.6 0.8 0.8 0 0 1 0-1.6z", "centre", true),
  ],

  // A flow inside a turn: the straight vector is what is beneath, the arc is
  // what the tool does to it. Not a bare ring, which reads as "reload".
  turn: [
    line("M17.4 6.6a7.6 7.6 0 1 0 2.4 6.2", "arc"),
    line("M14.1 6.9l3.3-0.3 0.3 3.3", "head"),
    line("M8.6 12h6.2m-1.8-1.8 1.8 1.8-1.8 1.8", "flow"),
  ],

  // Parallel flows pushed out of true, deepest in the middle: what a warp does
  // to the field it reads.
  warp: [
    line("M4 7.6c4-3.6 12 3.6 16 0", "upper"),
    line("M4 12c4-5.6 12 5.6 16 0", "middle"),
    line("M4 16.4c4-3.6 12 3.6 16 0", "lower"),
  ],
};

/** One tool's geometry. Every tool has some; this is for the tests. */
export function iconFor(tool: ActiveTool): Icon {
  return ICONS[tool];
}

/** Every tool the palette can draw, hand included. */
export const ICON_TOOLS = Object.keys(ICONS) as ActiveTool[];

export default function ToolIcon({ tool }: { tool: ActiveTool }): ReactElement {
  return (
    <svg
      className="tool-icon"
      viewBox="0 0 24 24"
      width="18"
      height="18"
      // Decorative: the button carries the name. Without this a screen reader
      // announces the button twice, once for the label and once for the graphic.
      aria-hidden="true"
      focusable="false"
    >
      {ICONS[tool]}
    </svg>
  );
}
