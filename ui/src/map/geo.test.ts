import { describe, expect, it } from "vitest";

import { EARTH_RADIUS_M, destination, distanceM, initialBearing } from "./geo";

/**
 * Checked against the same analytic references as `ve_core::geo`, which is the
 * authority. If these two drift apart, dragging a handle stops matching what
 * the document records.
 */
describe("distanceM", () => {
  it("matches analytic cases", () => {
    expect(distanceM({ lon: 0, lat: 0 }, { lon: 90, lat: 0 })).toBeCloseTo(
      (EARTH_RADIUS_M * Math.PI) / 2,
      3,
    );
    expect(distanceM({ lon: 0, lat: -90 }, { lon: 0, lat: 90 })).toBeCloseTo(
      EARTH_RADIUS_M * Math.PI,
      3,
    );
    expect(distanceM({ lon: 0, lat: 0 }, { lon: 0, lat: 1 })).toBeCloseTo(
      EARTH_RADIUS_M * (Math.PI / 180),
      6,
    );
  });

  it("treats the antimeridian as ordinary", () => {
    const twoDegrees = EARTH_RADIUS_M * 2 * (Math.PI / 180);
    expect(distanceM({ lon: 179, lat: 0 }, { lon: -179, lat: 0 })).toBeCloseTo(twoDegrees, 3);
  });
});

describe("initialBearing", () => {
  it("points the right way at the cardinals", () => {
    const origin = { lon: 0, lat: 0 };
    expect(initialBearing(origin, { lon: 0, lat: 10 })).toBeCloseTo(0, 9);
    expect(initialBearing(origin, { lon: 10, lat: 0 })).toBeCloseTo(90, 9);
    expect(initialBearing(origin, { lon: 0, lat: -10 })).toBeCloseTo(180, 9);
    expect(initialBearing(origin, { lon: -10, lat: 0 })).toBeCloseTo(270, 9);
  });

  /**
   * The reason this module exists. Screen space at 60°N is stretched in
   * longitude, so a bearing read off the screen would be wrong; the great
   * circle bulges poleward and the true bearing is north of east.
   */
  it("is a true bearing, not a screen angle", () => {
    const bearing = initialBearing({ lon: 0, lat: 60 }, { lon: 10, lat: 60 });
    expect(bearing).toBeLessThan(90);
    expect(bearing).toBeGreaterThan(80);
  });
});

describe("destination", () => {
  it("round trips with distance and bearing", () => {
    const start = { lon: -30, lat: 45 };
    const end = destination(start, 117, 1_234_567);
    expect(distanceM(start, end)).toBeCloseTo(1_234_567, 3);
    expect(initialBearing(start, end)).toBeCloseTo(117, 6);
  });

  it("wraps across the antimeridian", () => {
    const end = destination({ lon: 179, lat: 0 }, 90, EARTH_RADIUS_M * 2 * (Math.PI / 180));
    expect(end.lon).toBeCloseTo(-179, 6);
    expect(end.lat).toBeCloseTo(0, 9);
  });

  it("crosses the pole", () => {
    const end = destination({ lon: 0, lat: 89 }, 0, EARTH_RADIUS_M * 2 * (Math.PI / 180));
    expect(end.lat).toBeCloseTo(89, 6);
    expect(Math.abs(end.lon)).toBeCloseTo(180, 6);
  });

  /** A fixed distance is the same ground distance at any latitude. */
  it("is the same distance wherever it starts", () => {
    for (const lat of [0, 45, 70, 85]) {
      for (const bearing of [0, 90, 200]) {
        const start = { lon: 0, lat };
        expect(distanceM(start, destination(start, bearing, 500_000))).toBeCloseTo(500_000, 3);
      }
    }
  });
});
