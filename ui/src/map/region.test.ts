import { describe, expect, it } from "vitest";

import type { Region } from "./region";
import { normalizeLon } from "./camera";
import {
  editsRegion,
  recentred,
  regionGesture,
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

describe("recentred", () => {
  it("moves a rectangle and a disc by their own centre", () => {
    // A capture's region is placed per frame and the map has to draw it where
    // the frame it is showing puts it (spec.md 8.7, M16).
    const rect = recentred(
      { kind: "rect", centre: [0, 0], halfWidthDeg: 5, halfHeightDeg: 3 },
      20,
      -10,
    );
    expect(rect).toEqual({
      kind: "rect",
      centre: [20, -10],
      halfWidthDeg: 5,
      halfHeightDeg: 3,
    });
    expect(recentred({ kind: "disc", centre: [1, 2], radiusDeg: 4 }, -30, 60)).toEqual({
      kind: "disc",
      centre: [-30, 60],
      radiusDeg: 4,
    });
  });

  it("moves a polygon by its bounding-box centre, as the backend does", () => {
    // `RegionShape::anchor` uses the bounding box, not the mean of the
    // vertices. The mean would be a different point, and a dense corner would
    // drag the region off the pointer.
    const dense: [number, number][] = [
      [0, 0],
      [0.1, 0],
      [0.2, 0],
      [10, 10],
    ];
    const moved = recentred({ kind: "polygon", points: dense }, 100, 100);
    expect(moved.kind).toBe("polygon");
    if (moved.kind !== "polygon") return;
    // The box centre was (5, 5), so every point moves by (95, 95).
    expect(moved.points[0]).toEqual([95, 95]);
    expect(moved.points[3]).toEqual([105, 105]);
  });

  it("keeps a polygon's shape across the seam", () => {
    // A ring straddling 180 is stored with raw longitudes either side of it —
    // 179 and -179 — and moving it shifts every one by the same amount. What
    // has to survive is the *shape*: each edge, measured the short way round,
    // is the edge it was. Comparing the raw spread instead would say a
    // two-degree ring is 358 degrees wide, which is the seam and not the ring.
    const ring: [number, number][] = [
      [179, 0],
      [-179, 0],
      [-179, 2],
      [179, 2],
    ];
    const edges = (points: ReadonlyArray<readonly [number, number]>) =>
      points.map((point, i) => {
        const next = points[(i + 1) % points.length]!;
        return [normalizeLon(next[0] - point[0]), next[1] - point[1]];
      });

    const moved = recentred({ kind: "polygon", points: ring }, 0, 0);
    if (moved.kind !== "polygon") return;
    expect(edges(moved.points)).toEqual(edges(ring));

    // And the ring really has moved: its bounding-box centre was on the seam
    // and is now at the origin, which is what the backend was told.
    const shift = normalizeLon(moved.points[0]![0] - ring[0]![0]);
    expect(shift).toBeCloseTo(normalizeLon(-180), 9);
  });
});

describe("regionGesture", () => {
  // The operators have no shape_source, so on their side an extent can only
  // mean a disc: a rectangle therefore goes as its four corners, which is
  // exact, and only a circle goes as an extent (spec.md 8.2).
  it("sends a rectangle as a ring of its corners", () => {
    const gesture = regionGesture({
      kind: "rect",
      centre: [10, 20],
      halfWidthDeg: 5,
      halfHeightDeg: 2,
    });
    expect(gesture.kind).toBe("ring");
    if (gesture.kind !== "ring") return;
    expect(gesture.points).toHaveLength(4);
    expect(gesture.points).toContainEqual([5, 18]);
    expect(gesture.points).toContainEqual([15, 22]);
  });

  it("sends a circle as an extent with its rim due north", () => {
    const gesture = regionGesture({ kind: "disc", centre: [0, 0], radiusDeg: 3 });
    expect(gesture).toEqual({ kind: "extent", centre: [0, 0], rim: [0, 3] });
  });

  it("sends a polygon as itself", () => {
    const points: [number, number][] = [
      [0, 0],
      [4, 0],
      [2, 3],
    ];
    expect(regionGesture({ kind: "polygon", points })).toEqual({ kind: "ring", points });
  });

  it("names the brush and the four operators, and not the ones measured from an anchor", () => {
    for (const tool of ["brush", "mask", "intensity", "divergence", "turn"]) {
      expect(editsRegion(tool), tool).toBe(true);
    }
    // The clone stamp and the warp read from their anchor to somewhere else,
    // and a region does not say where; the shape fill is drawn with its own
    // presets rather than applied to a selection.
    for (const tool of ["clone_stamp", "warp", "shape_fill", "curve"]) {
      expect(editsRegion(tool), tool).toBe(false);
    }
  });
});
