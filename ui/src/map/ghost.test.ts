import { describe, expect, it } from "vitest";

import { applyMatrix, ghostMatrix, ghostRegion } from "./ghost";

/**
 * The transform is checked against the property that defines it — where it
 * puts points — and never against a second copy of its own arithmetic.
 */
describe("the drag's ghost transform", () => {
  const at = (x: number, y: number) => ({ x, y });
  const place = (x: number, y: number, rotationDeg = 0, radiusM = 1000) => ({
    pivot: at(x, y),
    rotationDeg,
    radiusM,
  });

  it("is the identity when the drag has gone nowhere", () => {
    const m = ghostMatrix(place(300, 200), place(300, 200));
    const moved = applyMatrix(m, at(317, 189));
    expect(moved.x).toBeCloseTo(317, 9);
    expect(moved.y).toBeCloseTo(189, 9);
  });

  it("carries every point by the same offset for a move", () => {
    const m = ghostMatrix(place(300, 200), place(380, 150));
    for (const point of [at(300, 200), at(260, 240), at(400, 100)]) {
      const moved = applyMatrix(m, point);
      expect(moved.x).toBeCloseTo(point.x + 80, 9);
      expect(moved.y).toBeCloseTo(point.y - 50, 9);
    }
  });

  /**
   * Screen y runs down, so a quarter turn clockwise on the map takes a point
   * due east of the pivot to one due south of it. Getting the sign wrong spins
   * the ghost the wrong way, which looks deliberate and is not.
   */
  it("turns clockwise about the destination pivot", () => {
    const m = ghostMatrix(place(300, 200, 0), place(300, 200, 90));
    const moved = applyMatrix(m, at(340, 200));
    expect(moved.x).toBeCloseTo(300, 6);
    expect(moved.y).toBeCloseTo(240, 6);
  });

  it("scales distance from the pivot by the change in reach", () => {
    const m = ghostMatrix(place(300, 200, 0, 1000), place(300, 200, 0, 2500));
    const moved = applyMatrix(m, at(340, 180));
    expect(moved.x).toBeCloseTo(300 + 40 * 2.5, 6);
    expect(moved.y).toBeCloseTo(200 - 20 * 2.5, 6);
  });

  it("turns and scales about the pivot it is being dragged to, not the one it left", () => {
    const m = ghostMatrix(place(100, 100, 0, 1000), place(400, 300, 180, 2000));
    // The pivot itself lands on the new pivot whatever else the drag does.
    const pivot = applyMatrix(m, at(100, 100));
    expect(pivot.x).toBeCloseTo(400, 6);
    expect(pivot.y).toBeCloseTo(300, 6);
    // A point 10 px east of the old pivot ends 20 px west of the new one.
    const east = applyMatrix(m, at(110, 100));
    expect(east.x).toBeCloseTo(380, 6);
    expect(east.y).toBeCloseTo(300, 6);
  });

  it("scales by one rather than by nothing when the selection has no reach", () => {
    const m = ghostMatrix(place(100, 100, 0, 0), place(200, 100, 0, 0));
    const moved = applyMatrix(m, at(110, 100));
    expect(moved.x).toBeCloseTo(210, 9);
    expect(moved.y).toBeCloseTo(100, 9);
  });
});

describe("the square of screen a ghost is taken from", () => {
  const canvas = { width: 800, height: 600 };

  it("covers the reach on both axes", () => {
    const region = ghostRegion({ x: 400, y: 300 }, { rx: 50, ry: 20 }, canvas, 0);
    expect(region).toEqual({ left: 350, top: 280, width: 100, height: 40 });
  });

  it("is clamped to the canvas, so a half-visible selection still copies", () => {
    const region = ghostRegion({ x: 10, y: 300 }, { rx: 50, ry: 20 }, canvas, 0);
    expect(region).toEqual({ left: 0, top: 280, width: 60, height: 40 });
  });

  it("is nothing at all when the selection is off screen", () => {
    expect(ghostRegion({ x: -200, y: 300 }, { rx: 50, ry: 20 }, canvas, 0)).toBeNull();
    expect(ghostRegion({ x: 400, y: 900 }, { rx: 50, ry: 20 }, canvas, 0)).toBeNull();
  });
});
