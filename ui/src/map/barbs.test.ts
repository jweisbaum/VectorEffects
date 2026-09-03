import { describe, expect, it } from "vitest";

import { barbElementCount, barbElements } from "./barbs";

describe("barbElements", () => {
  /**
   * Checked against the conventional 5 / 10 / 50 knot encoding. These are the
   * values a reference chart shows, not values read back out of the code.
   */
  it("matches the reference encoding", () => {
    const cases: Array<[number, number, number, number]> = [
      // knots, pennants, full, half
      [0, 0, 0, 0],
      [2, 0, 0, 0],
      [5, 0, 0, 1],
      [10, 0, 1, 0],
      [15, 0, 1, 1],
      [20, 0, 2, 0],
      [25, 0, 2, 1],
      [45, 0, 4, 1],
      [50, 1, 0, 0],
      [55, 1, 0, 1],
      [65, 1, 1, 1],
      [100, 2, 0, 0],
      [105, 2, 0, 1],
    ];
    for (const [knots, pennants, full, half] of cases) {
      const got = barbElements(knots);
      expect({ knots, ...got }).toEqual({
        knots,
        pennants,
        full,
        half,
        calm: knots < 2.5,
      });
    }
  });

  it("rounds to the nearest five knots before decomposing", () => {
    // 13 kt is nearer 15 than 10, so it must show a full barb and a half.
    expect(barbElements(13)).toMatchObject({ full: 1, half: 1 });
    expect(barbElements(12)).toMatchObject({ full: 1, half: 0 });
    expect(barbElements(17.4)).toMatchObject({ full: 1, half: 1 });
    expect(barbElements(18)).toMatchObject({ full: 2, half: 0 });
  });

  it("treats very light wind as calm", () => {
    expect(barbElements(0).calm).toBe(true);
    expect(barbElements(2.4).calm).toBe(true);
    expect(barbElements(2.6).calm).toBe(false);
  });

  it("rejects nonsense rather than drawing garbage", () => {
    expect(barbElements(Number.NaN).calm).toBe(true);
    expect(barbElements(-5).calm).toBe(true);
  });

  it("reconstructs the rounded speed from its elements", () => {
    for (let knots = 5; knots <= 150; knots += 5) {
      const e = barbElements(knots);
      expect(e.pennants * 50 + e.full * 10 + e.half * 5).toBe(knots);
    }
  });

  it("stays within the shader vertex budget", () => {
    for (let knots = 0; knots <= 200; knots += 1) {
      expect(barbElementCount(barbElements(knots))).toBeLessThanOrEqual(8);
    }
  });
});
