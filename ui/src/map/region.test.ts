import { describe, expect, it } from "vitest";

import type { Region } from "./region";
import {
  fillGesture,
  regionBounds,
  regionContains,
  regionFromDrag,
  regionFromLasso,
  regionOfView,
  regionRing,
  wholeMap,
} from "./region";

const camera = (centerLon: number, centerLat: number, pxPerDeg: number) => ({
  centerLon,
  centerLat,
  pxPerDeg,
});

describe("regionFromDrag", () => {
  it("draws a rectangle corner to corner", () => {
    const region = regionFromDrag("rect", [10, 20], [30, 40]);
    expect(region).toEqual({
      kind: "rect",
      centre: [20, 30],
      halfWidthDeg: 10,
      halfHeightDeg: 10,
    });
  });

  it("draws a circle from its centre, as the shape fill's presets are", () => {
    const region = regionFromDrag("circle", [0, 0], [3, 4]);
    expect(region).toEqual({ kind: "disc", centre: [0, 0], radiusDeg: 5 });
  });

  it("takes the short way across the antimeridian", () => {
    // 170° to -170° is 20° of drag, not 340°.
    const region = regionFromDrag("rect", [170, 0], [-170, 10]);
    expect(region?.kind).toBe("rect");
    if (region?.kind !== "rect") throw new Error("rect");
    expect(region.halfWidthDeg).toBeCloseTo(10);
    expect(region.centre[0]).toBeCloseTo(180 === region.centre[0] ? 180 : -180);
  });

  it("is nothing for a click", () => {
    expect(regionFromDrag("rect", [10, 20], [10, 20])).toBeNull();
    expect(regionFromDrag("circle", [10, 20], [10, 20])).toBeNull();
  });
});

describe("regionFromLasso", () => {
  it("needs three vertices to have an inside", () => {
    expect(regionFromLasso([[0, 0]])).toBeNull();
    expect(
      regionFromLasso([
        [0, 0],
        [1, 0],
      ]),
    ).toBeNull();
    expect(
      regionFromLasso([
        [0, 0],
        [1, 0],
        [0, 1],
      ]),
    ).not.toBeNull();
  });
});

describe("regionOfView", () => {
  it("has the viewport's corners", () => {
    // 800 x 400 px at 10 px per degree is 80° by 40°.
    const region = regionOfView(camera(20, 30, 10), 800, 400);
    expect(region).toEqual({
      kind: "rect",
      centre: [20, 30],
      halfWidthDeg: 40,
      halfHeightDeg: 20,
    });
  });

  it("clamps in latitude: there is no ground past the pole", () => {
    const region = regionOfView(camera(0, 80, 10), 800, 400);
    if (region.kind !== "rect") throw new Error("rect");
    expect(region.centre[1] + region.halfHeightDeg).toBeCloseTo(90);
    expect(region.centre[1] - region.halfHeightDeg).toBeCloseTo(60);
  });

  it("is the whole map when the viewport is wider than the world", () => {
    expect(regionOfView(camera(0, 0, 1), 1000, 200)).toEqual(wholeMap());
  });
});

describe("regionRing", () => {
  it("gives a rectangle its four corners", () => {
    expect(
      regionRing({ kind: "rect", centre: [0, 0], halfWidthDeg: 2, halfHeightDeg: 1 }),
    ).toEqual([
      [-2, -1],
      [2, -1],
      [2, 1],
      [-2, 1],
    ]);
  });

  it("flattens a circle smoothly", () => {
    const ring = regionRing({ kind: "disc", centre: [0, 0], radiusDeg: 1 });
    expect(ring).toHaveLength(64);
    for (const [lon, lat] of ring) expect(Math.hypot(lon, lat)).toBeCloseTo(1);
  });
});

describe("regionBounds", () => {
  it("keeps a region's width across the antimeridian", () => {
    // A 20° box centred on the seam: the bounds run 170 to 190, not -180 to 180.
    const bounds = regionBounds({
      kind: "rect",
      centre: [180, 0],
      halfWidthDeg: 10,
      halfHeightDeg: 5,
    });
    expect(bounds.east - bounds.west).toBeCloseTo(20);
    expect(bounds.north).toBeCloseTo(5);
    expect(bounds.south).toBeCloseTo(-5);
  });
});

describe("regionContains", () => {
  it("holds what is inside a rectangle and not what is outside", () => {
    const region: Region = {
      kind: "rect",
      centre: [0, 0],
      halfWidthDeg: 10,
      halfHeightDeg: 5,
    };
    expect(regionContains(region, 5, 2)).toBe(true);
    expect(regionContains(region, 15, 2)).toBe(false);
    expect(regionContains(region, 5, 7)).toBe(false);
  });

  it("is not confused by the seam", () => {
    const region: Region = {
      kind: "rect",
      centre: [180, 0],
      halfWidthDeg: 10,
      halfHeightDeg: 5,
    };
    expect(regionContains(region, 175, 0)).toBe(true);
    expect(regionContains(region, -175, 0)).toBe(true);
    expect(regionContains(region, 160, 0)).toBe(false);
  });

  it("holds a polygon's inside", () => {
    const region = regionFromLasso([
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ]);
    if (!region) throw new Error("a region");
    expect(regionContains(region, 5, 5)).toBe(true);
    expect(regionContains(region, 15, 5)).toBe(false);
  });

  it("is round on the map at every latitude", () => {
    // A map-space circle is a circle on screen wherever it is drawn: the same
    // degree radius holds at the equator and at 70° N (D55).
    for (const lat of [0, 45, 70]) {
      const region: Region = { kind: "disc", centre: [0, lat], radiusDeg: 5 };
      expect(regionContains(region, 4.9, lat)).toBe(true);
      expect(regionContains(region, 5.1, lat)).toBe(false);
      expect(regionContains(region, 0, lat + 4.9)).toBe(true);
      expect(regionContains(region, 0, lat + 5.1)).toBe(false);
    }
  });
});

describe("fillGesture", () => {
  it("makes a rectangle region the shape fill's rectangle preset", () => {
    const { gesture, shapeSource } = fillGesture({
      kind: "rect",
      centre: [10, 20],
      halfWidthDeg: 3,
      halfHeightDeg: 2,
    });
    expect(shapeSource).toBe(2);
    expect(gesture).toEqual({ kind: "extent", centre: [10, 20], rim: [13, 22] });
  });

  it("makes a circle region its circle preset, rim due north", () => {
    // Due north keeps the two half-extents equal, which is what the backend's
    // hypot reads back as the radius.
    const { gesture, shapeSource } = fillGesture({
      kind: "disc",
      centre: [0, 45],
      radiusDeg: 4,
    });
    expect(shapeSource).toBe(3);
    expect(gesture).toEqual({ kind: "extent", centre: [0, 45], rim: [0, 49] });
  });

  it("makes a lasso its freehand polygon", () => {
    const points: Array<[number, number]> = [
      [0, 0],
      [1, 0],
      [1, 1],
    ];
    const { gesture, shapeSource } = fillGesture({ kind: "polygon", points });
    expect(shapeSource).toBe(0);
    expect(gesture).toEqual({ kind: "ring", points });
  });
});
