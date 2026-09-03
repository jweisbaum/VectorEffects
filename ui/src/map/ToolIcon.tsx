/**
 * The palette's icons.
 *
 * Inline SVG, drawn here rather than pulled from an icon set: invariant 5 for-
 * bids any runtime network access, so a webfont or a CDN sprite is out, and a
 * bundled icon package would be a dependency carried for seven glyphs. These
 * are seven paths.
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

import { HAND, type ActiveTool } from "./tools";

/** One icon: the geometry inside a `0 0 24 24` box. */
type Icon = readonly ReactElement[];

/**
 * A rounded stroke, which is every line in the set.
 *
 * Given as a component rather than repeated per path so the seven icons cannot
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

  // An eraser block on its edge, with the line it has cleared beneath it.
  eraser: [
    line("M8.6 17.4 4.9 13.7a1.7 1.7 0 0 1 0-2.4l7.4-7.4a1.7 1.7 0 0 1 2.4 0l4.8 4.8a1.7 1.7 0 0 1 0 2.4l-6 6z", "block"),
    line("M4 20.4h16", "swept"),
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
