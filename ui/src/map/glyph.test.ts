import { describe, expect, it } from "vitest";

import { arrowGeometry, barbGeometry, glyphLayout, latticeUnderStroke } from "./glyph";

/** A stroke width; only the calm marker's size depends on it. */
const WIDTH = 2;

describe("barbGeometry", () => {
  /**
   * 65 knots is a pennant, a full barb and a half barb -- the standard
   * worked example from a barb chart. The pennant is filled, the other two
   * are strokes, and the shaft is a stroke of its own.
   */
  it("decomposes 65 knots into a pennant, a barb and a half barb", () => {
    const { lines, fills } = barbGeometry(0, 65, 100, 40, WIDTH);
    expect(fills).toHaveLength(1);
    expect(fills[0]).toHaveLength(3);
    expect(lines).toHaveLength(3);
  });

  /** The staff points at where the wind is coming from, not where it goes. */
  it("puts the staff upwind of the station", () => {
    // Blowing towards the east, so it comes from the west: the staff runs
    // west, which is -x on screen.
    const { lines } = barbGeometry(90, 25, 100, 40, WIDTH);
    const [, tip] = lines[0]!;
    expect(tip![0]).toBeCloseTo(-100, 6);
    expect(tip![1]).toBeCloseTo(0, 6);
  });

  /**
   * Flags sit on the poleward side of the staff, which mirrors across the
   * equator. Screen y is down, so poleward is -y in the north and +y in the
   * south.
   */
  it("mirrors the flags across the equator", () => {
    const north = barbGeometry(90, 10, 100, 40, WIDTH);
    const south = barbGeometry(90, 10, 100, -40, WIDTH);
    const flagTip = (g: typeof north) => g.lines[1]![1]![1];
    const anchorY = north.lines[1]![0]![1];
    expect(flagTip(north)).toBeLessThan(anchorY);
    expect(flagTip(south)).toBeGreaterThan(anchorY);
  });

  /** Below 2.5 knots the mark is a bare closed shape, with no flags at all. */
  it("draws calm as a closed marker with no flags", () => {
    const { lines, fills } = barbGeometry(90, 1, 100, 40, WIDTH);
    expect(fills).toHaveLength(0);
    expect(lines).toHaveLength(1);
    const marker = lines[0]!;
    expect(marker[0]).toEqual(marker[marker.length - 1]);
  });

  /** Rounding to the nearest 5 knots is the convention, not an accident. */
  it("rounds to the nearest five knots before decomposing", () => {
    // 13 kt rounds to 15: one full barb and one half barb.
    const { lines, fills } = barbGeometry(0, 13, 100, 40, WIDTH);
    expect(fills).toHaveLength(0);
    expect(lines).toHaveLength(3);
  });
});

describe("arrowGeometry", () => {
  /** The head is downwind: an arrow shows where the air is going. */
  it("puts the head downwind", () => {
    // Towards 180 is south, which is +y on screen.
    const { lines, fills } = arrowGeometry(180, 100);
    const [tail] = lines[0]!;
    const [head] = fills[0]!;
    expect(head![1]).toBeGreaterThan(tail![1]);
    expect(head![1]).toBeCloseTo(50, 6);
  });
});

describe("glyphLayout", () => {
  /**
   * The lattice step is chosen so the on-screen spacing clears the target;
   * at 4 px per degree a 46 px barb target needs at least 11.5 degrees, and
   * the ladder's next step up is 15.
   */
  it("chooses a step that clears the target spacing", () => {
    const { stepDeg, spacing } = glyphLayout("barb", 4, 1);
    expect(stepDeg).toBe(15);
    expect(spacing).toBe(60);
  });

  /** A retina display doubles the device-pixel target, so the lattice coarsens. */
  it("coarsens with the pixel ratio", () => {
    expect(glyphLayout("barb", 4, 2).stepDeg).toBeGreaterThan(
      glyphLayout("barb", 4, 1).stepDeg,
    );
  });
});

describe("latticeUnderStroke", () => {
  /** Half a degree of latitude, so a stamp reaches half a lattice cell at 1°. */
  const HALF_DEGREE_KM = 111.19492664455873 / 2;

  it("covers nothing for an empty stroke", () => {
    expect(latticeUnderStroke([], HALF_DEGREE_KM, 1, 100)).toEqual([]);
  });

  /**
   * Points sit on the globally anchored grid -- multiples of the step from
   * (-180, 90) -- not on the stroke. A stamp centred at 10.2 E, 20.2 N with
   * half a degree of reach takes in the whole-degree point at 10 E, 20 N and
   * nothing else.
   */
  it("snaps to the globe's lattice, not to the stroke", () => {
    expect(latticeUnderStroke([[10.2, 20.2]], HALF_DEGREE_KM, 1, 100)).toEqual([
      [10, 20],
    ]);
  });

  /** Out of reach of every lattice point is a legitimate answer, not a fallback. */
  it("covers nothing when the brush falls between lattice points", () => {
    expect(latticeUnderStroke([[10.5, 20.5]], HALF_DEGREE_KM / 10, 1, 100)).toEqual([]);
  });

  /** The whole swept path, not just its ends. */
  it("covers the lattice along a stroke", () => {
    const found = latticeUnderStroke(
      [
        [0, 0],
        [4, 0],
      ],
      HALF_DEGREE_KM,
      1,
      100,
    );
    expect(found).toEqual([
      [0, 0],
      [1, 0],
      [2, 0],
      [3, 0],
      [4, 0],
    ]);
  });

  /** A ground circle spans more longitude the further from the equator. */
  it("reaches further in longitude near a pole", () => {
    // At 60 N a degree of longitude is half a degree of latitude on the
    // ground, so a stamp that reaches one lattice point at the equator...
    expect(latticeUnderStroke([[0, 0]], HALF_DEGREE_KM * 1.5, 1, 100)).toEqual([[0, 0]]);
    // ...takes in its neighbours either side up there.
    expect(latticeUnderStroke([[0, 60]], HALF_DEGREE_KM * 1.5, 1, 100)).toEqual([
      [-1, 60],
      [0, 60],
      [1, 60],
    ]);
  });

  /** A square brush covers its corners; a round one of the same size does not. */
  it("covers the corners of a square stamp", () => {
    const round = latticeUnderStroke([[0.5, 0.5]], HALF_DEGREE_KM * 1.2, 1, 100);
    const square = latticeUnderStroke(
      [[0.5, 0.5]],
      HALF_DEGREE_KM * 1.2,
      1,
      100,
      "square",
    );
    expect(round).toEqual([]);
    expect(square).toHaveLength(4);
  });

  /** The lattice wraps, so a stamp on the dateline reaches both sides of it. */
  it("wraps across the antimeridian", () => {
    // 0.4 degrees short of the dateline, reaching 0.75 either way: the point on
    // the far side of it is the nearer of the two.
    const found = latticeUnderStroke([[179.6, 0]], HALF_DEGREE_KM * 1.5, 1, 100);
    expect(found).toEqual([
      [179, 0],
      [-180, 0],
    ]);
  });

  it("stops at the limit", () => {
    expect(
      latticeUnderStroke(
        [
          [0, 0],
          [40, 0],
        ],
        HALF_DEGREE_KM,
        1,
        7,
      ),
    ).toHaveLength(7);
  });
});
