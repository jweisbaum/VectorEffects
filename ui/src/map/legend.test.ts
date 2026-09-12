/**
 * The legend's numbers.
 */
import { describe, expect, it } from "vitest";

import { legendKnots } from "./legend";

describe("the legend's numbers", () => {
  /**
   * The report: the ends should say what is on screen. Whole knots could not —
   * a current runs at a knot or two, so both ends rounded to the same number
   * and "0 to 1" was the whole of what the legend said about a field with a
   * tenth of a knot at one end and nine tenths at the other.
   */
  it("gives a narrow span the precision to distinguish its ends", () => {
    const span = 0.87 - 0.12;
    expect(legendKnots(0.12, span)).toBe("0.12");
    expect(legendKnots(0.87, span)).toBe("0.87");
  });

  /** A middling span reads in tenths: hundredths would be noise at that width. */
  it("gives a middling span tenths", () => {
    const span = 9.8 - 1.2;
    expect(legendKnots(1.2, span)).toBe("1.2");
    expect(legendKnots(9.8, span)).toBe("9.8");
  });

  /** And a wide one whole knots, where a tenth is below what the eye reads. */
  it("gives a wide span whole knots", () => {
    expect(legendKnots(0, 47)).toBe("0");
    expect(legendKnots(46.6, 47)).toBe("47");
  });

  /**
   * The step comes from the span and not from the kind, so a slow *wind* is
   * treated like a slow current rather than rounded away.
   */
  it("chooses by span rather than by how fast the field is", () => {
    expect(legendKnots(30.25, 0.5)).toBe("30.25");
    expect(legendKnots(0.4, 60)).toBe("0");
  });

  /** Trailing zeroes claim a precision the scale does not have. */
  it("drops trailing zeroes and the point with them", () => {
    expect(legendKnots(0.5, 1)).toBe("0.5");
    expect(legendKnots(2, 1)).toBe("2");
    expect(legendKnots(3, 10)).toBe("3");
  });
});
