import { describe, expect, it } from "vitest";

import type { ProjectRegion } from "../generated/ProjectRegion";
import {
  cameraForProjection,
  clampCamera,
  normalizeLon,
  panBy,
  project,
  projectionFor,
  zoomAbout,
  type Camera,
  type Viewport,
} from "./camera";
import {
  clampToRegion,
  regionCentre,
  regionContains,
  regionFitPxPerDeg,
  regionOutline,
  regionRings,
} from "./extent";

const pacific: ProjectRegion = { west: 160, east: 200, south: -10, north: 10, full_circle: false };
const arctic: ProjectRegion = { west: -180, east: 180, south: 60, north: 90, full_circle: true };
const view: Viewport = { width: 800, height: 600 };

describe("the region outline", () => {
  it("runs the rectangle's edges unwrapped, closed", () => {
    const ring = regionOutline(pacific, 2);
    expect(ring[0]).toEqual({ lon: 160, lat: -10 });
    expect(ring[ring.length - 1]).toEqual(ring[0]);
    // Unwrapped: the eastern edge sits at 200, not -160.
    expect(Math.max(...ring.map((p) => p.lon))).toBe(200);
    // No step longer than the asked one.
    for (let i = 1; i < ring.length; i++) {
      const a = ring[i - 1]!, b = ring[i]!;
      expect(Math.hypot(a.lon - b.lon, a.lat - b.lat)).toBeLessThanOrEqual(2 + 1e-9);
    }
  });

  it("is one closed parallel for a cap", () => {
    const ring = regionOutline(arctic, 2);
    expect(ring.every((p) => p.lat === 60)).toBe(true);
    expect(ring[0]!.lon).toBe(-180);
    expect(ring[ring.length - 1]!.lon).toBe(180);
  });
});

describe("regionRings", () => {
  const closed = (ring: { lon: number; lat: number }[]) => {
    const a = ring[0]!, b = ring[ring.length - 1]!;
    // A parallel closes at the same place one turn round: -180 and 180.
    return a.lat === b.lat && (a.lon === b.lon || Math.abs(b.lon - a.lon) === 360);
  };

  it("is one ring for a box, one for a cap and two for a band, each closed", () => {
    const band: ProjectRegion = { west: -180, east: 180, south: -30, north: 40, full_circle: true };
    const antarctic: ProjectRegion = { west: -180, east: 180, south: -90, north: -55, full_circle: true };
    const cases: [ProjectRegion, number[]][] = [
      [pacific, [-10]], [arctic, [60]], [antarctic, [-55]], [band, [-30, 40]],
    ];
    for (const [region, lats] of cases) {
      const rings = regionRings(region, 2);
      expect(rings).toHaveLength(lats.length);
      rings.forEach((ring, i) => {
        expect(closed(ring)).toBe(true);
        expect(ring[0]!.lat).toBe(lats[i]);
      });
    }
  });

  it("is what the outline concatenates", () => {
    const band: ProjectRegion = { west: -180, east: 180, south: -30, north: 40, full_circle: true };
    expect(regionOutline(band, 5)).toEqual(regionRings(band, 5).flat());
    expect(regionOutline(pacific, 5)).toEqual(regionRings(pacific, 5)[0]);
  });
});

describe("regionContains", () => {
  it("wraps the antimeridian", () => {
    expect(regionContains(pacific, 179, 0)).toBe(true);
    expect(regionContains(pacific, -165, 5)).toBe(true);
    expect(regionContains(pacific, -150, 0)).toBe(false);
    expect(regionContains(pacific, 170, 11)).toBe(false);
  });

  it("holds every longitude of a cap, the pole included", () => {
    expect(regionContains(arctic, -179.9, 61)).toBe(true);
    expect(regionContains(arctic, 37, 90)).toBe(true);
    expect(regionContains(arctic, 37, 59)).toBe(false);
  });
});

describe("regionCentre", () => {
  it("is the middle of the arc, across the antimeridian", () => {
    expect(regionCentre(pacific)).toEqual({ lon: -180, lat: 0 });
  });
  it("is the pole for a cap", () => {
    expect(regionCentre(arctic).lat).toBe(90);
  });
});

describe("the camera held to the region (R8)", () => {
  it("cannot zoom out past a region", () => {
    const cam = clampCamera({ centerLon: 180, centerLat: 0, pxPerDeg: 0.1, region: pacific }, view);
    expect(cam.pxPerDeg).toBeCloseTo(Math.min(800 / 40, 600 / 20), 3);
  });

  it("pans across the antimeridian inside a region and stops at its edge", () => {
    let cam = clampCamera({ centerLon: 175, centerLat: 0, pxPerDeg: 100, region: pacific }, view);
    cam = panBy(cam, view, 2000, 0); // 20° east: past 180
    expect(normalizeLon(cam.centerLon)).toBeCloseTo(-165, 6);
    cam = panBy(cam, view, 5000, 0); // far past the east edge
    expect(normalizeLon(cam.centerLon + 400 / cam.pxPerDeg)).toBeCloseTo(-160, 6);
  });

  it("stops panning at the region's south edge", () => {
    let cam = clampCamera({ centerLon: 180, centerLat: 0, pxPerDeg: 100, region: pacific }, view);
    cam = panBy(cam, view, 0, 5000);
    expect(cam.centerLat - 300 / cam.pxPerDeg).toBeCloseTo(-10, 6);
  });

  it("a region narrower than the window is centred", () => {
    const thin = { west: 10, east: 12, south: 0, north: 40, full_circle: false };
    const cam = clampCamera({ centerLon: 50, centerLat: 20, pxPerDeg: 15, region: thin }, view);
    expect(cam.centerLon).toBeCloseTo(11, 6);
  });

  it("an arctic cap wraps in longitude and stops at 60N", () => {
    const cam = clampCamera({ centerLon: 170, centerLat: 0, pxPerDeg: 10, region: arctic }, view);
    expect(cam.centerLon).toBeCloseTo(170, 6); // free wrap
    // y increases northward, so the window's southern edge is half a window
    // below the centre in the projection's own coordinate.
    expect(projectionFor(cam).latOf(projectionFor(cam).yOf(cam.centerLat) - 300 / cam.pxPerDeg))
      .toBeGreaterThanOrEqual(60 - 1e-6);
    // A cap cannot fit horizontally: the floor fits its latitude alone.
    expect(cam.pxPerDeg).toBeCloseTo(600 / 30, 6);
  });

  it("stops a Mercator cap at the projection's own edge", () => {
    const cam = clampCamera({ centerLon: 0, centerLat: 89, pxPerDeg: 0.1, region: arctic, projection: "mercator" }, view);
    const p = projectionFor(cam);
    const top = p.yOf(cam.centerLat) + 300 / cam.pxPerDeg;
    const bottom = p.yOf(cam.centerLat) - 300 / cam.pxPerDeg;
    expect(top).toBeLessThanOrEqual(p.yOf(p.maxLat) + 1e-6);
    expect(p.latOf(bottom)).toBeGreaterThanOrEqual(60 - 1e-6);
  });

  it("under the globe the centre stays in the region", () => {
    const cam = clampCamera({ centerLon: 0, centerLat: 0, pxPerDeg: 50, region: pacific, projection: "orthographic" }, view);
    expect(regionContains(pacific, cam.centerLon, cam.centerLat)).toBe(true);
    expect(cam.pxPerDeg).toBeGreaterThanOrEqual(
      regionFitPxPerDeg(pacific, view, projectionFor(cam), { lon: cam.centerLon, lat: cam.centerLat }) - 1e-9,
    );
  });

  it("an arctic cap under polar stereographic fits around the pole", () => {
    const cam = clampCamera({ centerLon: 0, centerLat: 0, pxPerDeg: 0.01, region: arctic, projection: "stereographic" }, view);
    expect(cam.centerLat).toBeCloseTo(90, 6);
    expect(cam.pxPerDeg).toBeCloseTo(regionFitPxPerDeg(arctic, view, projectionFor(cam), { lon: 0, lat: 90 }), 3);
  });

  it("lets a zoomed-in globe move off the pole inside the cap", () => {
    const cam = clampCamera({ centerLon: 30, centerLat: 70, pxPerDeg: 50, region: arctic, projection: "orthographic" }, view);
    expect(cam.centerLat).toBeCloseTo(70, 6);
    expect(cam.centerLon).toBeCloseTo(30, 6);
  });

  it("a global camera is untouched", () => {
    const cam: Camera = { centerLon: 10, centerLat: 20, pxPerDeg: 10 };
    // No region: the very same object back, not a copy.
    expect(clampToRegion(cam, view)).toBe(cam);
    // Inside the world's limits (floor min(800/360, 600/180) = 2.2 px/°; the
    // window is 60° tall, so the centre may sit anywhere within ±60°), the
    // whole clamp hands back the same centre and zoom and adds no region.
    expect(clampCamera(cam, view)).toEqual({ centerLon: 10, centerLat: 20, pxPerDeg: 10 });
  });

  it("keeps the region through a zoom and a change of projection", () => {
    const cam: Camera = clampCamera({ centerLon: 180, centerLat: 0, pxPerDeg: 30, region: pacific }, view);
    expect(zoomAbout(cam, view, { x: 100, y: 100 }, 0.01).region).toBe(pacific);
    const atlantic: ProjectRegion = { west: -60, east: -10, south: 20, north: 50, full_circle: false };
    const robinson = cameraForProjection({ ...cam, region: atlantic }, view, "robinson");
    expect(robinson.region).toBe(atlantic);
    expect(regionContains(atlantic, robinson.centerLon, robinson.centerLat)).toBe(true);
  });
});

/**
 * On each screen axis the projected region either covers the window or is
 * centred in it: the R8 rule, read off the screen rather than the plane.
 */
function coversOrCentres(cam: Camera, region: ProjectRegion): void {
  const points = regionOutline(region, 1).map((p) => project(cam, view, p)).filter((p) => Number.isFinite(p.x));
  expect(points.length).toBeGreaterThan(10);
  const xs = points.map((p) => p.x), ys = points.map((p) => p.y);
  for (const [lo, hi, size] of [
    [Math.min(...xs), Math.max(...xs), view.width],
    [Math.min(...ys), Math.max(...ys), view.height],
  ] as const) {
    if (hi - lo >= size - 1e-3) {
      expect(lo).toBeLessThanOrEqual(1e-3);
      expect(hi).toBeGreaterThanOrEqual(size - 1e-3);
    } else {
      expect((lo + hi) / 2).toBeCloseTo(size / 2, 3);
    }
  }
}

describe("fixed general maps hold the region's projected box", () => {
  const europe: ProjectRegion = { west: -20, east: 40, south: 30, north: 70, full_circle: false };
  const britain: ProjectRegion = { west: -12, east: 4, south: 48, north: 62, full_circle: false };

  it("Robinson: zoomed right out, the box fits and is centred", () => {
    const cam = clampCamera({ centerLon: 120, centerLat: -40, pxPerDeg: 0.01, region: europe, projection: "robinson" }, view);
    coversOrCentres(cam, europe);
  });

  it("Robinson: zoomed in and looking off the edge, the box still covers", () => {
    const cam = clampCamera({ centerLon: 100, centerLat: -30, pxPerDeg: 40, region: europe, projection: "robinson" }, view);
    coversOrCentres(cam, europe);
    // And a pan towards the edge stops there.
    coversOrCentres(panBy(cam, view, 3000, -2000), europe);
  });

  it("British National Grid (EPSG:27700) holds a British region", () => {
    const out = clampCamera({ centerLon: -2, centerLat: 54, pxPerDeg: 0.01, region: britain, projection: "epsg_27700" }, view);
    coversOrCentres(out, britain);
    let cam = clampCamera({ ...out, pxPerDeg: 200 }, view);
    cam = panBy(cam, view, 90000, -90000);
    coversOrCentres(cam, britain);
  });
});
