import { describe, expect, it } from "vitest";

import { PROJECTIONS, projectionOf, worldHeightDeg } from "./projection";

describe("every projection", () => {
  it("round-trips a latitude back to itself", () => {
    // `unproject` is on the *painting* path, not just the readout: a stroke
    // drawn at 60°N has to land at 60°N (M11's acceptance).
    for (const projection of PROJECTIONS) {
      for (const lat of [-80, -60, -23.5, 0, 23.5, 45, 60, 80]) {
        if (Math.abs(lat) > projection.maxLat) continue;
        const back = projection.latOf(projection.yOf(lat));
        expect(back, `${projection.id} at ${lat}`).toBeCloseTo(lat, 6);
      }
    }
  });

  it("leaves the equator where it is", () => {
    // Every one of these agrees at the equator, which is what lets `pxPerDeg`
    // keep meaning what it meant.
    for (const projection of PROJECTIONS) {
      expect(projection.yOf(0), projection.id).toBeCloseTo(0, 12);
      expect(projection.latOf(0), projection.id).toBeCloseTo(0, 12);
    }
  });

  it("is symmetric about the equator", () => {
    for (const projection of PROJECTIONS) {
      for (const lat of [10, 45, 70]) {
        if (lat > projection.maxLat) continue;
        expect(projection.yOf(-lat), projection.id).toBeCloseTo(-projection.yOf(lat), 9);
      }
    }
  });

  it("increases northward", () => {
    for (const projection of PROJECTIONS) {
      let previous = -Infinity;
      for (let lat = -80; lat <= 80; lat += 10) {
        const y = projection.yOf(lat);
        expect(y, `${projection.id} at ${lat}`).toBeGreaterThan(previous);
        previous = y;
      }
    }
  });
});

describe("equirectangular", () => {
  it("is the identity, so nothing about the app's numbers changes", () => {
    const projection = projectionOf("equirectangular");
    for (const lat of [-90, -45, 0, 45, 90]) {
      expect(projection.yOf(lat)).toBe(lat);
      expect(projection.latOf(lat)).toBe(lat);
    }
    expect(worldHeightDeg(projection)).toBe(180);
  });
});

describe("mercator", () => {
  it("stops short of the pole, because tan does not", () => {
    const projection = projectionOf("mercator");
    expect(projection.maxLat).toBeLessThan(90);
    // Clamped rather than infinite: a NaN here would take the whole map out.
    expect(Number.isFinite(projection.yOf(90))).toBe(true);
    expect(Number.isFinite(projection.yOf(-90))).toBe(true);
  });

  it("makes the world square at its own limit", () => {
    // The Web Mercator limit is chosen so the world is 360 by 360.
    expect(worldHeightDeg(projectionOf("mercator"))).toBeCloseTo(360, 3);
  });

  it("stretches the high latitudes, which is the whole point", () => {
    const projection = projectionOf("mercator");
    // A degree at 60°N covers twice the vertical span of one at the equator:
    // sec(60°) = 2.
    const atEquator = projection.yOf(1) - projection.yOf(0);
    const atSixty = projection.yOf(61) - projection.yOf(60);
    expect(atSixty / atEquator).toBeCloseTo(2, 1);
  });
});

describe("miller", () => {
  it("reaches the poles, which is what it is for", () => {
    const projection = projectionOf("miller");
    expect(projection.maxLat).toBe(90);
    expect(Number.isFinite(projection.yOf(90))).toBe(true);
    expect(projection.latOf(projection.yOf(90))).toBeCloseTo(90, 6);
  });

  it("stretches less than Mercator", () => {
    const miller = projectionOf("miller");
    const mercator = projectionOf("mercator");
    expect(miller.yOf(70)).toBeLessThan(mercator.yOf(70));
  });
});

describe("projectionOf", () => {
  it("falls back rather than failing on an unknown id", () => {
    // A settings file can hold anything; a map that will not draw is worse
    // than one drawn the old way.
    expect(projectionOf("nonsense" as never).id).toBe("equirectangular");
  });
});

describe("scaleAt", () => {
  it("is the derivative of yOf, checked against a finite difference", () => {
    // The overlay draws a footprint's vertical radius through this, so a
    // scale that is not the derivative is a brush that draws the wrong size.
    // Checked against yOf rather than restated, so the two cannot drift.
    const h = 1e-5;
    for (const projection of PROJECTIONS) {
      for (const lat of [-70, -40, -10, 0, 15, 45, 70]) {
        const slope = (projection.yOf(lat + h) - projection.yOf(lat - h)) / (2 * h);
        expect(projection.scaleAt(lat), `${projection.id} at ${lat}`).toBeCloseTo(slope, 6);
      }
    }
  });

  it("is 1 at the equator, where every projection agrees", () => {
    for (const projection of PROJECTIONS) {
      expect(projection.scaleAt(0), projection.id).toBeCloseTo(1, 12);
    }
  });
});
