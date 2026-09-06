import { describe, expect, it } from "vitest";

import { positionOf, valueOf } from "./sliderMath";

describe("a centred slider", () => {
  it("puts zero in the middle whatever the two ends are", () => {
    expect(positionOf(0, -100, 200)).toBe(0);
    expect(positionOf(-100, -100, 200)).toBe(-1);
    expect(positionOf(200, -100, 200)).toBe(1);
    expect(positionOf(100, -100, 200)).toBeCloseTo(0.5, 12);
    expect(positionOf(-50, -100, 200)).toBeCloseTo(-0.5, 12);
  });

  it("round-trips through the track", () => {
    for (const value of [-100, -37, 0, 12.5, 150, 200]) {
      expect(valueOf(positionOf(value, -100, 200), -100, 200)).toBeCloseTo(value, 9);
    }
    for (const value of [-400, -1, 0, 3, 400]) {
      expect(valueOf(positionOf(value, -400, 400), -400, 400)).toBeCloseTo(value, 9);
    }
  });

  it("is one straight line for a range that does not straddle zero", () => {
    expect(positionOf(5, 0, 10)).toBe(0);
    expect(valueOf(0, 0, 10)).toBe(5);
    expect(valueOf(-1, 2, 4)).toBe(2);
    expect(valueOf(1, 2, 4)).toBe(4);
  });

  it("clamps a thumb pushed past an end", () => {
    expect(valueOf(3, -100, 200)).toBe(200);
    expect(valueOf(-3, -100, 200)).toBe(-100);
  });
});
