/**
 * The palette's icons.
 *
 * A picture cannot be checked by a test for whether it *looks* like a brush.
 * What can be checked is everything around it: that every tool has one, that no
 * two share a shape by copy-paste, that the geometry is real, and that the
 * icons carry no colour of their own — the failures that ship a blank button, a
 * duplicated one, or a set that goes invisible when the theme changes.
 *
 * That every tool has one is not a test at all but a type: `ICONS` is a
 * `Record` over the tool union, so a tool added to the catalogue fails to
 * compile until it has an icon. This checks the half that types cannot.
 */
import { describe, expect, it } from "vitest";

import { ICON_TOOLS, iconFor } from "./ToolIcon";
import { FILL, HAND, MEASURE, SELECT } from "./tools";

/** The attributes a `<path>` in the set carries. */
interface PathProps {
  d: string;
  fill: string;
  stroke: string;
  strokeWidth: number;
}

/** One icon's paths, typed: `ReactElement` says nothing about its props. */
function shapes(tool: (typeof ICON_TOOLS)[number]): PathProps[] {
  return iconFor(tool).map((element) => element.props as PathProps);
}

/** The `d` attributes of one icon's paths. */
function paths(tool: (typeof ICON_TOOLS)[number]): string[] {
  return shapes(tool).map((shape) => shape.d);
}

describe("the icon set", () => {
  /**
   * Sixteen: the hand, the two region tools of 8.2, the measure tool of 10,
   * the six tools of spec 6.2, the four modifiers of 6.3, and the patch of 8.5
   * and the macro of 8.7 — neither drawn from the palette, and both needing a
   * mark all the same. Named rather than counted against the palette, because
   * the palette arrives over IPC and a test that compared the two would only
   * be comparing the frontend to itself.
   */
  it("covers the hand, the region and measure tools and every drawing tool", () => {
    expect([...ICON_TOOLS].sort()).toEqual(
      [
        "brush",
        "circle",
        "clone_stamp",
        "curve",
        "divergence",
        "mask",
        HAND,
        "intensity",
        "shape_fill",
        "turn",
        "warp",
        SELECT,
        FILL,
        // Draws no object at all: it lays a measurement over the map
        // (spec.md 10, M8).
        MEASURE,
        // Neither is in the palette, but every object is named by its tool
        // and the panels draw a mark beside it (spec.md 8.5, 8.7).
        "patch",
        "macro",
      ].sort(),
    );
  });

  it("gives every tool some geometry", () => {
    for (const tool of ICON_TOOLS) {
      const drawn = paths(tool);
      expect(drawn.length, `${tool} has no paths`).toBeGreaterThan(0);
      for (const d of drawn) {
        expect(d, `${tool} has an empty path`).toMatch(/^[Mm]/);
        expect(d.length, `${tool} has a path too short to be a shape`).toBeGreaterThan(8);
      }
    }
  });

  /**
   * Two tools sharing a shape is the copy-paste failure: the set still renders,
   * every button still works, and two of them are indistinguishable.
   */
  it("draws no two tools the same", () => {
    const seen = new Map<string, string>();
    for (const tool of ICON_TOOLS) {
      const shape = paths(tool).join("|");
      const already = seen.get(shape);
      expect(already, `${tool} and ${already} are the same drawing`).toBeUndefined();
      seen.set(shape, tool);
    }
  });

  /**
   * The icons take the button's colour, so the active state and any future
   * theme reach them for free. A hard-coded fill or stroke would survive every
   * visual check until the day the button's background moved under it.
   */
  it("takes its colour from the button", () => {
    for (const tool of ICON_TOOLS) {
      for (const { fill, stroke } of shapes(tool)) {
        expect(stroke, `${tool} hard-codes a stroke`).toBe("currentColor");
        expect(["none", "currentColor"], `${tool} hard-codes a fill`).toContain(fill);
      }
    }
  });

  /** One weight across the set, or the row reads as seven unrelated marks. */
  it("is drawn at one weight", () => {
    const weights = new Set<number>();
    for (const tool of ICON_TOOLS) {
      for (const { strokeWidth } of shapes(tool)) weights.add(strokeWidth);
    }
    expect(weights.size).toBe(1);
  });

  /**
   * The absolute points are inside the 24-unit box.
   *
   * Only the absolute commands — `M` and `L` — are checked, because they are
   * the only numbers in a path that are certainly coordinates: a lowercase
   * command's numbers are deltas, and an arc carries radii and flags among its
   * own. A parser would answer for all of them; this answers for the ones that
   * need no parser, which is enough to catch the mistake that actually happens,
   * an icon drawn on somebody else's grid.
   */
  it("places its absolute points inside its box", () => {
    for (const tool of ICON_TOOLS) {
      for (const d of paths(tool)) {
        const points = [...d.matchAll(/[ML]\s*(-?[\d.]+)[\s,]+(-?[\d.]+)/g)];
        expect(points.length, `${tool} has a path with no absolute point`).toBeGreaterThan(0);
        for (const [, x, y] of points) {
          for (const value of [Number(x), Number(y)]) {
            expect(value, `${tool} places a point at ${value}`).toBeGreaterThanOrEqual(0);
            expect(value, `${tool} places a point at ${value}`).toBeLessThanOrEqual(24);
          }
        }
      }
    }
  });
});
