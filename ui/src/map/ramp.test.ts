import { describe, expect, it } from "vitest";

import { RAMP_COLOURS, RAMP_STOPS, rampColour, rampCss } from "./ramp";

describe("rampColour", () => {
  /**
   * The legend draws a CSS gradient over `RAMP_STOPS` while the brush preview
   * samples `rampColour`. If the two disagree the legend stops describing what
   * the map paints, which is the whole reason it exists.
   */
  it("agrees with the legend's gradient at every stop", () => {
    const last = RAMP_COLOURS.length - 1;
    RAMP_STOPS.forEach((css, index) => {
      const [r, g, b] = rampColour(index / last);
      const channels = [r, g, b].map((c) => Math.round(c * 255));
      const hex = `#${channels.map((c) => c.toString(16).padStart(2, "0")).join("")}`;
      expect(hex).toBe(css);
    });
  });

  it("clamps outside the range rather than extrapolating", () => {
    expect(rampColour(-1)).toEqual(RAMP_COLOURS[0]);
    expect(rampColour(2)).toEqual(RAMP_COLOURS[RAMP_COLOURS.length - 1]);
  });

  /** Halfway between two stops is the midpoint of their channels. */
  it("interpolates linearly between stops", () => {
    const last = RAMP_COLOURS.length - 1;
    const [r] = rampColour(0.5 / last);
    expect(r).toBeCloseTo((RAMP_COLOURS[0]![0] + RAMP_COLOURS[1]![0]) / 2, 12);
  });
});

describe("rampCss", () => {
  /** Calm fades out, so a calm field leaves the basemap readable. */
  it("fades calm to nothing", () => {
    expect(rampCss(0, 30)).toMatch(/, 0\.000\)$/);
  });

  /** Above a few percent of the range the ramp reaches its full opacity. */
  it("reaches full opacity well below the top of the range", () => {
    expect(rampCss(30 * 0.06, 30)).toMatch(/, 0\.720\)$/);
  });

  /** A preview needs to be visible even when the field it previews would not be. */
  it("honours a minimum alpha", () => {
    expect(rampCss(0, 30, 0.28)).toMatch(/, 0\.280\)$/);
    // Never dims a colour that is already stronger than the floor.
    expect(rampCss(30, 30, 0.28)).toMatch(/, 0\.720\)$/);
  });
});
