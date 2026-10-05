/**
 * Where the region lies on screen, which the overlay dims the outside of
 * (spec 5.1, R8). Checked against the camera's own projection of the
 * region's corners and against points known to be inside and outside it,
 * across the antimeridian and round both poles.
 */
import { describe, expect, it } from "vitest";

import type { ProjectRegion } from "../generated/ProjectRegion";
import { clampCamera, project, type Camera, type ScreenPoint, type Viewport } from "./camera";
import { regionMask } from "./regionMask";

const pacific: ProjectRegion = { west: 160, east: 200, south: -10, north: 10, full_circle: false };
const arctic: ProjectRegion = { west: -180, east: 180, south: 60, north: 90, full_circle: true };
const antarctic: ProjectRegion = { west: -180, east: 180, south: -90, north: -60, full_circle: true };
const view: Viewport = { width: 1600, height: 900 };

/** Even-odd crossing count: inside the rings, as the fill reads them. */
function inside(rings: ScreenPoint[][], p: ScreenPoint): boolean {
  let odd = false;
  for (const ring of rings) {
    for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
      const a = ring[i]!, b = ring[j]!;
      if (a.y > p.y !== b.y > p.y && p.x < ((b.x - a.x) * (p.y - a.y)) / (b.y - a.y) + a.x) odd = !odd;
    }
  }
  return odd;
}

/** The mask of a camera clamped to `region` at its minimum zoom. */
function atMinimum(region: ProjectRegion, projection: Camera["projection"], lon = 0, lat = 0) {
  const camera = clampCamera({ centerLon: lon, centerLat: lat, pxPerDeg: 0.01, region, ...(projection ? { projection } : {}) }, view);
  return { camera, mask: regionMask(camera, view)! };
}

describe("the region on screen", () => {
  it("is nothing for a global project", () => {
    expect(regionMask({ centerLon: 0, centerLat: 0, pxPerDeg: 4 }, view)).toBeNull();
  });

  it("is the rectangle between the corners under equirectangular, across the antimeridian", () => {
    const { camera, mask } = atMinimum(pacific, undefined, 180);
    expect(mask.closed).toBe(true);
    const sw = project(camera, view, { lon: 160, lat: -10 });
    const ne = project(camera, view, { lon: -160, lat: 10 });
    const xs = mask.rings[0]!.map((p) => p.x), ys = mask.rings[0]!.map((p) => p.y);
    expect(Math.min(...xs)).toBeCloseTo(sw.x, 6);
    expect(Math.max(...xs)).toBeCloseTo(ne.x, 6);
    expect(Math.max(...ys)).toBeCloseTo(sw.y, 6);
    expect(Math.min(...ys)).toBeCloseTo(ne.y, 6);
    expect(inside(mask.rings, project(camera, view, { lon: 179, lat: 0 }))).toBe(true);
    expect(inside(mask.rings, project(camera, view, { lon: -179, lat: 0 }))).toBe(true);
  });

  it("finds the near copy whichever side of the antimeridian the camera is on", () => {
    for (const lon of [165, -165]) {
      // 80 px a degree: the window is 20° of the region's 40°, held inside it.
      const camera = clampCamera({ centerLon: lon, centerLat: 0, pxPerDeg: 80, region: pacific }, view);
      const mask = regionMask(camera, view)!;
      const xs = mask.rings[0]!.map((p) => p.x);
      expect(Math.min(...xs)).toBeLessThanOrEqual(1e-6);
      expect(Math.max(...xs)).toBeGreaterThanOrEqual(view.width - 1e-6);
      expect(inside(mask.rings, { x: view.width / 2, y: view.height / 2 })).toBe(true);
    }
  });

  it("holds a polar cap to its parallel under Mercator, stopping at the map's edge", () => {
    const { camera, mask } = atMinimum(arctic, "mercator");
    expect(mask.closed).toBe(true);
    const edge = project(camera, view, { lon: 0, lat: 60 }).y;
    expect(Math.max(...mask.rings[0]!.map((p) => p.y))).toBeCloseTo(Math.min(edge, view.height + 4), 6);
    expect(inside(mask.rings, project(camera, view, { lon: 20, lat: 75 }))).toBe(true);
  });

  for (const projection of ["stereographic", "orthographic"] as const) {
    for (const [name, cap, pole] of [["arctic", arctic, 90], ["antarctic", antarctic, -90]] as const) {
      it(`is a circle about the pole for the ${name} cap under ${projection}`, () => {
        const { camera, mask } = atMinimum(cap, projection, 0, pole);
        expect(mask.closed).toBe(true);
        expect(mask.rings).toHaveLength(1);
        const centre = project(camera, view, { lon: 0, lat: pole });
        const radii = mask.rings[0]!.map((p) => Math.hypot(p.x - centre.x, p.y - centre.y));
        expect(Math.max(...radii) - Math.min(...radii)).toBeLessThan(0.5);
        expect(inside(mask.rings, centre)).toBe(true);
        expect(inside(mask.rings, { x: 2, y: 2 })).toBe(false);
      });
    }
  }

  it("is whole under the globe for the Pacific box", () => {
    const { camera, mask } = atMinimum(pacific, "orthographic", 180);
    expect(mask.closed).toBe(true);
    expect(inside(mask.rings, project(camera, view, { lon: -179, lat: 0 }))).toBe(true);
    expect(inside(mask.rings, project(camera, view, { lon: 150, lat: 0 }))).toBe(false);
  });

  it("draws a cap under Robinson down to its pole line", () => {
    const { camera, mask } = atMinimum(antarctic, "robinson");
    expect(mask.closed).toBe(true);
    expect(inside(mask.rings, project(camera, view, { lon: 120, lat: -80 }))).toBe(true);
    expect(inside(mask.rings, project(camera, view, { lon: -170, lat: -65 }))).toBe(true);
    expect(inside(mask.rings, project(camera, view, { lon: 0, lat: -50 }))).toBe(false);
  });

  /** A box more than a hemisphere wide cannot be whole on a globe. */
  it("is outlined, not filled, when a horizon cuts it", () => {
    const wide: ProjectRegion = { west: -170, east: 170, south: -60, north: 60, full_circle: false };
    const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 3, region: wide, projection: "orthographic" };
    const mask = regionMask(camera, view)!;
    expect(mask.closed).toBe(false);
    expect(mask.rings.length).toBeGreaterThan(0);
  });

  /**
   * Zoomed in far enough that one 2° segment of the outline is longer than
   * half the window's diagonal: the walk must still find the edge where it
   * crosses the window, and not mistake the long off-screen chords for a seam.
   */
  for (const projection of ["orthographic", "stereographic"] as const) {
    it(`keeps the Pacific box whole under ${projection} at 2,000 px a degree, its east edge on screen`, () => {
      const camera = clampCamera({ centerLon: -160.3, centerLat: 0, pxPerDeg: 2000, region: pacific, projection }, view);
      const mask = regionMask(camera, view)!;
      expect(mask.closed).toBe(true);
      const edge = project(camera, view, { lon: -160, lat: 0 });
      expect(edge.x).toBeGreaterThan(0);
      expect(edge.x).toBeLessThan(view.width);
      // An edge piece crosses the window: samples on screen, on either side
      // of the centre line, along the east edge.
      const onScreen = mask.rings.flat().filter((p) => p.x >= 0 && p.x <= view.width && p.y >= 0 && p.y <= view.height);
      expect(onScreen.some((p) => p.y < view.height / 2)).toBe(true);
      expect(onScreen.some((p) => p.y > view.height / 2)).toBe(true);
      expect(inside(mask.rings, { x: edge.x - 50, y: edge.y })).toBe(true);
      expect(inside(mask.rings, { x: edge.x + 50, y: edge.y })).toBe(false);
    });
  }

  it("keeps a box whole under Robinson zoomed in at its edge, and still sees the seam", () => {
    const atlantic: ProjectRegion = { west: -60, east: -10, south: 20, north: 50, full_circle: false };
    const camera = clampCamera({ centerLon: -10.2, centerLat: 35, pxPerDeg: 2000, region: atlantic, projection: "robinson" }, view);
    const mask = regionMask(camera, view)!;
    expect(mask.closed).toBe(true);
    const edge = project(camera, view, { lon: -10, lat: 35 });
    expect(inside(mask.rings, { x: edge.x - 50, y: edge.y })).toBe(true);
    expect(inside(mask.rings, { x: edge.x + 50, y: edge.y })).toBe(false);
    // Robinson's seam is the antimeridian, which the Pacific box crosses.
    expect(atMinimum(pacific, "robinson", 180).mask.closed).toBe(false);
  });
});
