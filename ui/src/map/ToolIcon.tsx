/**
 * The palette's icons.
 *
 * Inline SVG, drawn here rather than pulled from an icon set: invariant 5 for-
 * bids any runtime network access, so a webfont or a CDN sprite is out, and a
 * bundled icon package would be a dependency carried for a dozen glyphs. These
 * are a dozen sets of paths.
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

import { CAPTURE, HAND, INSERT, MEASURE, SELECT, type ActiveTool } from "./tools";

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
  // The capture tool: a frame with a record dot in it. Recording a run of
  // frames is what it does, and a bare dot would read as a brush.
  [CAPTURE]: [
    line("M4.5 6.5h15a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-15a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1z", "frame"),
    line("M12 9.2a2.8 2.8 0 1 1 0 5.6 2.8 2.8 0 0 1 0-5.6z", "dot", true),
  ],
  // The insert tool: a stamp coming down onto a baseline, which is what
  // putting a captured run of frames back on the map looks like.
  [INSERT]: [
    line("M8 3.6h8a1 1 0 0 1 1 1v5.4a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1V4.6a1 1 0 0 1 1-1z", "block"),
    line("M12 12.2v4.4", "stem"),
    line("M9.4 14.6 12 17.2l2.6-2.6", "point"),
    line("M4.5 20.4h15", "ground"),
  ],
  // A pair of dividers, the instrument the tool is named for: two legs from a
  // hinge, points down on the chart.
  [MEASURE]: [
    line("M12 3.6a1.6 1.6 0 1 1 0 3.2 1.6 1.6 0 0 1 0-3.2z", "hinge"),
    line("M11.2 6.6 6.4 20.4", "left-leg"),
    line("M12.8 6.6 17.6 20.4", "right-leg"),
    line("M9.6 13.2h4.8", "brace"),
  ],
  // The hand that pans: an open hand, palm out, four fingers and a thumb.
  // The old one was a palm with three fingers and read as a pointer; a hand
  // that pans is the whole hand (M24).
  [HAND]: [
    line(
      "M7.2 12.6V6.4a1.3 1.3 0 0 1 2.6 0v5.2m0-6.6a1.3 1.3 0 0 1 2.6 0v6.2m0-5.2a1.3 1.3 0 0 1 2.6 0v5.6m0-3.8a1.3 1.3 0 0 1 2.6 0v6.4a6.2 6.2 0 0 1-6.2 6.2h-1a6.2 6.2 0 0 1-5-2.5L3.1 13.9a1.5 1.5 0 0 1 2.3-1.9l1.8 2.2",
      "hand",
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

  // A liquify: one flow dragged along by a hand, the pull deepest where the
  // stroke went. The warp's family, with the smear told from the block.
  liquify: [
    line("M4 8h6c3 0 3 8 6 8h4", "dragged"),
    line("M4 16h5.5", "left"),
    line("M13.5 6.5 16 9l-2.5 2.5", "hand"),
  ],
  // Parallel flows pushed out of true, deepest in the middle: what a warp does
  // to the field it reads.
  warp: [
    line("M4 7.6c4-3.6 12 3.6 16 0", "upper"),
    line("M4 12c4-5.6 12 5.6 16 0", "middle"),
    line("M4 16.4c4-3.6 12 3.6 16 0", "lower"),
  ],
};

/**
 * The eyedropper, for the option bar's sample button (M24): a dropper held
 * at an angle, bulb up, tip down where the sample is taken.
 *
 * Not in `ICONS`: it is not a tool, and the record over the tool union has
 * to stay exactly the tools.
 */
export const EYEDROPPER_ICON: Icon = [
  line("M15.8 3.6a2.2 2.2 0 0 1 3.1 0l1.5 1.5a2.2 2.2 0 0 1 0 3.1l-2.2 2.2-4.6-4.6z", "bulb", true),
  line("M13.6 5.8 18.2 10.4 9.4 19.2a2 2 0 0 1-1.4.6H5.6l-1.4 1.4-1-1 1.4-1.4V16.4a2 2 0 0 1 .6-1.4z", "barrel"),
  line("M12 7.4 16.6 12", "tip"),
];

/** Undo: an arrow curling back to the left (M25). */
export const UNDO_ICON: Icon = [
  line("M8.4 7.2 4.6 11l3.8 3.8", "head"),
  line("M4.8 11h9a5.2 5.2 0 0 1 0 10.4H10", "arrow"),
];

/** Redo: the same arrow, curling forward to the right. */
export const REDO_ICON: Icon = [
  line("M15.6 7.2 19.4 11l-3.8 3.8", "head"),
  line("M19.2 11h-9a5.2 5.2 0 0 0 0 10.4H14", "arrow"),
];

/** Loop: two arrows chasing each other round a ring, for the transport. */
export const LOOP_ICON: Icon = [
  line("M17.6 8.4A7 7 0 0 0 5.4 10", "top"),
  line("M6.4 15.6A7 7 0 0 0 18.6 14", "bottom"),
  line("M4.6 6.2v4.2h4.2", "top-head"),
  line("M19.4 17.8v-4.2h-4.2", "bottom-head"),
];

/** An icon as an SVG, for a button that is not a tool. */
export function IconSvg({ icon, size = 18 }: { icon: Icon; size?: number }): ReactElement {
  return (
    <svg
      className="tool-icon"
      viewBox="0 0 24 24"
      width={size}
      height={size}
      aria-hidden="true"
      focusable="false"
    >
      {icon}
    </svg>
  );
}

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
