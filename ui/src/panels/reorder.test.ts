import { describe, expect, it } from "vitest";

import { dropSide, layerDropIndex } from "./reorder";

/** The order the panel shows, top-first, after `MoveLayer { from, to }` on `["A", "B", "C"]`. */
function shown(from: number, to: number): string[] {
  const layers = ["A", "B", "C"];
  const [moved] = layers.splice(from, 1);
  layers.splice(to, 0, moved as string);
  return [...layers].reverse();
}

describe("layerDropIndex", () => {
  // Document ["A", "B", "C"] is shown C, B, A.
  it("puts the bottom layer above the top one", () => {
    expect(shown(0, layerDropIndex(0, 2, true))).toEqual(["A", "C", "B"]);
  });

  it("puts the bottom layer just below the top one", () => {
    expect(shown(0, layerDropIndex(0, 2, false))).toEqual(["C", "A", "B"]);
  });

  it("puts the top layer just above the bottom one", () => {
    expect(shown(2, layerDropIndex(2, 0, true))).toEqual(["B", "C", "A"]);
  });

  it("puts the top layer below the bottom one", () => {
    expect(shown(2, layerDropIndex(2, 0, false))).toEqual(["B", "A", "C"]);
  });

  it("leaves a layer dropped on itself where it is", () => {
    expect(layerDropIndex(1, 1, true)).toBe(1);
    expect(layerDropIndex(1, 1, false)).toBe(1);
  });
});

describe("dropSide", () => {
  /**
   * The halves of the row, and the whole row between them: a drop anywhere
   * on it lands on one side or the other, so there is nowhere on a row that
   * a layer cannot be dropped beside (M33).
   */
  it("splits a row down the middle, with the midpoint below", () => {
    expect(dropSide(100, 100, 20)).toBe("above");
    expect(dropSide(109, 100, 20)).toBe("above");
    expect(dropSide(110, 100, 20)).toBe("below");
    expect(dropSide(119, 100, 20)).toBe("below");
  });
});
