/**
 * The selection box's hit test.
 */
import { describe, expect, it } from "vitest";

import { boxTakes, normalised, type DragBox } from "./boxSelect";

const PX = 10;
const HALF = 11;

describe("the selection box", () => {
  /**
   * The property that matters: which way the drag was made cannot change what
   * it caught. Every one of the four corners-first orderings of the same
   * rectangle has to take the same keys.
   */
  it("catches the same keys whichever way it was dragged", () => {
    const corners: DragBox[] = [
      { x0: 15, y0: 5, x1: 65, y1: 40 },
      { x0: 65, y0: 5, x1: 15, y1: 40 },
      { x0: 15, y0: 40, x1: 65, y1: 5 },
      { x0: 65, y0: 40, x1: 15, y1: 5 },
    ];
    for (const box of corners) {
      // Steps sit at (step + 0.5) * PX, so 1 through 6 lie in [15, 65] and
      // step 0, at 5, does not. Both edges are inclusive: a key exactly under
      // the corner was inside the rectangle the user drew.
      const caught = [0, 1, 2, 3, 4, 5, 6, 7].filter((step) => boxTakes(box, step, PX, 20, HALF));
      expect(caught, JSON.stringify(box)).toEqual([1, 2, 3, 4, 5, 6]);
      expect(normalised(box)).toEqual({ x0: 15, y0: 5, x1: 65, y1: 40 });
    }
  });

  /** A row is a band, not a line: a box that crosses it takes its keys. */
  it("takes a row the box merely crosses", () => {
    const box = { x0: 0, y0: 28, x1: 100, y1: 34 };
    expect(boxTakes(box, 3, PX, 20, HALF)).toBe(true);
    // ...and misses one it passes well clear of.
    expect(boxTakes(box, 3, PX, 60, HALF)).toBe(false);
  });

  /** A click is a box of no size, and takes nothing it is not on top of. */
  it("takes nothing from a press that did not move", () => {
    const box = { x0: 47, y0: 20, x1: 47, y1: 20 };
    expect([0, 1, 2, 3, 4, 5].some((step) => boxTakes(box, step, PX, 20, HALF))).toBe(false);
    // Squarely on a key's own pixel, it does take it — the same rule, not a
    // special case for clicks.
    expect(boxTakes({ x0: 45, y0: 20, x1: 45, y1: 20 }, 4, PX, 20, HALF)).toBe(true);
  });
});
