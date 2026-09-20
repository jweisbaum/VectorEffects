import { describe, expect, it } from "vitest";

import { PROJECTIONS, projectionOf } from "./projection";

import {
  MAX_PX_PER_DEG,
  MAX_TILE_LEVEL,
  TILE_SIZE,
  clampCamera,
  minPxPerDeg,
  normalizeLon,
  panBy,
  project,
  tileBounds,
  tileColumns,
  tileLevelFor,
  tileRows,
  unproject,
  visibleBounds,
  glyphLattice,
  glyphStepDegrees,
  visibleTiles,
  zoomAbout,
  type Camera,
  type Viewport,
} from "./camera";

const view: Viewport = { width: 1200, height: 800 };
const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 4 };

describe("normalizeLon", () => {
  it("folds into [-180, 180)", () => {
    expect(normalizeLon(181)).toBeCloseTo(-179, 9);
    expect(normalizeLon(-181)).toBeCloseTo(179, 9);
    expect(normalizeLon(180)).toBeCloseTo(-180, 9);
    expect(normalizeLon(-180)).toBeCloseTo(-180, 9);
    expect(normalizeLon(720 + 45)).toBeCloseTo(45, 9);
  });
});

describe("projection", () => {
  it("round trips", () => {
    for (const point of [
      { lon: 0, lat: 0 },
      { lon: -73.9, lat: 40.7 },
      { lon: 151.2, lat: -33.9 },
      { lon: 12, lat: 89 },
    ]) {
      const back = unproject(camera, view, project(camera, view, point));
      expect(back.lon).toBeCloseTo(point.lon, 6);
      expect(back.lat).toBeCloseTo(point.lat, 6);
    }
  });

  it("puts the camera centre at the viewport centre", () => {
    const p = project(camera, view, { lon: 0, lat: 0 });
    expect(p.x).toBeCloseTo(600, 9);
    expect(p.y).toBeCloseTo(400, 9);
  });

  it("increases x eastward and y southward", () => {
    const east = project(camera, view, { lon: 10, lat: 0 });
    const south = project(camera, view, { lon: 0, lat: -10 });
    expect(east.x).toBeGreaterThan(600);
    expect(south.y).toBeGreaterThan(400);
  });

  /// The acceptance criterion: crossing the dateline must not jump.
  it("is continuous across the antimeridian", () => {
    const near: Camera = { centerLon: 179, centerLat: 0, pxPerDeg: 4 };
    const before = project(near, view, { lon: 179.5, lat: 0 });
    const after = project(near, view, { lon: -179.5, lat: 0 });
    // 179.5 and -179.5 are one degree apart, so must be pxPerDeg apart.
    expect(Math.abs(after.x - before.x)).toBeCloseTo(4, 6);
  });

  it("keeps screen motion smooth while panning across the dateline", () => {
    let previous: number | null = null;
    for (let centerLon = 170; centerLon <= 190; centerLon += 1) {
      const c = clampCamera({ centerLon, centerLat: 0, pxPerDeg: 4 }, view);
      const p = project(c, view, { lon: 178, lat: 0 });
      if (previous !== null) {
        // One degree of pan is four pixels; a seam would show as a huge jump.
        expect(Math.abs(p.x - previous)).toBeLessThan(8);
      }
      previous = p.x;
    }
  });
});

describe("clampCamera", () => {
  it("stops the viewport scrolling past a pole", () => {
    const zoomed = clampCamera({ centerLon: 0, centerLat: 89, pxPerDeg: 20 }, view);
    const halfHeightDeg = view.height / 2 / 20;
    expect(zoomed.centerLat).toBeCloseTo(90 - halfHeightDeg, 6);

    const bounds = visibleBounds(zoomed, view);
    expect(bounds.north).toBeLessThanOrEqual(90.0001);
  });

  it("centres vertically when the whole world fits", () => {
    const out = clampCamera({ centerLon: 0, centerLat: 40, pxPerDeg: 1 }, view);
    expect(out.centerLat).toBe(0);
  });

  it("never zooms out past the whole world", () => {
    const out = clampCamera({ centerLon: 0, centerLat: 0, pxPerDeg: 0.0001 }, view);
    expect(out.pxPerDeg).toBeCloseTo(minPxPerDeg(view), 9);
  });

  it("lets longitude wrap freely", () => {
    expect(clampCamera({ ...camera, centerLon: 200 }, view).centerLon).toBeCloseTo(-160, 9);
  });
});

describe("zoomAbout", () => {
  it("keeps the point under the cursor fixed", () => {
    const anchor = { x: 900, y: 250 };
    const before = unproject(camera, view, anchor);
    const zoomed = zoomAbout(camera, view, anchor, 2);
    const after = unproject(zoomed, view, anchor);

    expect(zoomed.pxPerDeg).toBeCloseTo(8, 6);
    expect(normalizeLon(after.lon - before.lon)).toBeCloseTo(0, 4);
    expect(after.lat - before.lat).toBeCloseTo(0, 4);
  });

  it("anchors correctly across the dateline too", () => {
    const near: Camera = { centerLon: 179.5, centerLat: 10, pxPerDeg: 6 };
    const anchor = { x: 1100, y: 300 };
    const before = unproject(near, view, anchor);
    const after = unproject(zoomAbout(near, view, anchor, 1.5), view, anchor);
    expect(Math.abs(normalizeLon(after.lon - before.lon))).toBeLessThan(0.01);
  });
});

describe("tileLevelFor", () => {
  it("stays inside the pyramid", () => {
    for (const px of [0.01, 1, 4, 40, MAX_PX_PER_DEG, 100000]) {
      const z = tileLevelFor(px);
      expect(z).toBeGreaterThanOrEqual(0);
      expect(z).toBeLessThanOrEqual(MAX_TILE_LEVEL);
      expect(Number.isInteger(z)).toBe(true);
    }
  });

  it("never decreases as you zoom in", () => {
    let previous = -1;
    for (let px = 0.5; px <= MAX_PX_PER_DEG; px *= 1.3) {
      const z = tileLevelFor(px);
      expect(z).toBeGreaterThanOrEqual(previous);
      previous = z;
    }
  });

  /// Tiles must be at least as fine as the screen or the map looks soft.
  it("chooses tiles no coarser than the screen", () => {
    for (const px of [1, 3, 9, 40, 200, MAX_PX_PER_DEG]) {
      const z = tileLevelFor(px);
      const tilePxPerDeg = (tileColumns(z) * 256) / 360;
      expect(tilePxPerDeg).toBeGreaterThanOrEqual(px * 0.999);
    }
  });
});

describe("MAX_PX_PER_DEG", () => {
  /**
   * The cap belongs exactly where the pyramid runs out, and the pyramid is
   * the independent reference: `MAX_TILE_LEVEL` lays `tileColumns` tiles of
   * `TILE_SIZE` pixels across 360 degrees, so that is the finest scale at
   * which a tile pixel is still a screen pixel. Set lower, the map refuses to
   * zoom to levels the backend already renders; set higher, it zooms into
   * tiles coarser than the screen and goes soft.
   */
  it("stops exactly where the pyramid does", () => {
    expect(MAX_PX_PER_DEG).toBeCloseTo(
      (tileColumns(MAX_TILE_LEVEL) * TILE_SIZE) / 360,
      6,
    );
    expect(tileLevelFor(MAX_PX_PER_DEG)).toBe(MAX_TILE_LEVEL);
  });

  /** However far the map is asked to zoom, its tiles stay screen-sharp. */
  it("never lets the map outrun the pyramid", () => {
    for (const wanted of [MAX_PX_PER_DEG, MAX_PX_PER_DEG * 2, 1e6]) {
      const c = clampCamera({ centerLon: 0, centerLat: 0, pxPerDeg: wanted }, view);
      const tilePxPerDeg = (tileColumns(tileLevelFor(c.pxPerDeg)) * TILE_SIZE) / 360;
      expect(tilePxPerDeg).toBeGreaterThanOrEqual(c.pxPerDeg * 0.999);
    }
  });

  /**
   * What the zoom is *for*, against a real-world reference rather than the
   * constant: an S-57 band 6 cell is a berthing plan, and a berth is tens of
   * metres. One metre of latitude is one degree over
   * `2 * PI * EARTH_RADIUS_M / 360`, so the cap has to buy a pixel well under
   * the size of a boat's berth.
   */
  it("reaches berthing scale", () => {
    const metresPerDegree = (2 * Math.PI * 6_371_229) / 360;
    expect(metresPerDegree / MAX_PX_PER_DEG).toBeLessThan(25);
  });
});

describe("visibleTiles", () => {
  it("covers both sides of repeated worlds when tall projections fit vertically", () => {
    for (const projection of PROJECTIONS) for (const centerLon of [-179, 0, 179]) {
      const c: Camera = { centerLon, centerLat: 0, projection: projection.id,
        pxPerDeg: minPxPerDeg(view, projection) };
      const tiles = visibleTiles(c, view);
      const bounds = visibleBounds(c, view, true);
      expect(bounds.west).toBeCloseTo(centerLon - view.width / (2 * c.pxPerDeg), 8);
      for (const x of [1, view.width / 2, view.width - 1]) {
        const lon = centerLon + (x - view.width / 2) / c.pxPerDeg;
        expect(tiles.some(tile => {
          const b = tileBounds(tile.z, tile.x, tile.y);
          return lon >= b.west + tile.lonOffset && lon <= b.east + tile.lonOffset;
        }), `${projection.id} at pixel ${x}`).toBe(true);
      }
    }
  });

  it("returns addresses inside the pyramid", () => {
    for (const c of [
      camera,
      { centerLon: 179, centerLat: 60, pxPerDeg: 32 },
      { centerLon: -179, centerLat: -80, pxPerDeg: 64 },
    ]) {
      for (const tile of visibleTiles(clampCamera(c, view), view)) {
        expect(tile.x).toBeGreaterThanOrEqual(0);
        expect(tile.x).toBeLessThan(tileColumns(tile.z));
        expect(tile.y).toBeGreaterThanOrEqual(0);
        expect(tile.y).toBeLessThan(tileRows(tile.z));
      }
    }
  });

  it("covers the viewport", () => {
    const c = clampCamera({ centerLon: 20, centerLat: 30, pxPerDeg: 16 }, view);
    const tiles = visibleTiles(c, view);
    const bounds = visibleBounds(c, view);

    // Every corner of the view must fall inside some returned tile.
    const corners: Array<[number, number]> = [
      [bounds.west, bounds.north],
      [bounds.east, bounds.north],
      [bounds.west, bounds.south],
      [bounds.east, bounds.south],
    ];
    for (const [lon, lat] of corners) {
      const covered = tiles.some((t) => {
        const b = tileBounds(t.z, t.x, t.y);
        const west = b.west + t.lonOffset;
        const east = b.east + t.lonOffset;
        return lon >= west - 1e-9 && lon <= east + 1e-9 && lat <= b.north + 1e-9 && lat >= b.south - 1e-9;
      });
      expect(covered, `corner ${lon},${lat} uncovered`).toBe(true);
    }
  });

  it("emits wrapped copies rather than out-of-range columns", () => {
    const c = clampCamera({ centerLon: 179, centerLat: 0, pxPerDeg: 8 }, view);
    const tiles = visibleTiles(c, view);
    expect(tiles.some((t) => t.lonOffset !== 0)).toBe(true);
    for (const t of tiles) {
      expect([-720, -360, 0, 360, 720]).toContain(t.lonOffset);
    }
  });

  /**
   * Regression: a retina viewport at mid zoom wants 286 tiles. Truncating at
   * the budget left an unpainted block in the corner of the map.
   */
  it("always covers the viewport, even when the ideal level exceeds the budget", () => {
    const retina: Viewport = { width: 2880, height: 1684 };
    for (const c of [
      { centerLon: 180, centerLat: 20, pxPerDeg: 24 },
      { centerLon: 0, centerLat: 0, pxPerDeg: 60 },
      { centerLon: -40, centerLat: 35, pxPerDeg: 120 },
    ]) {
      const camera = clampCamera(c, retina);
      const tiles = visibleTiles(camera, retina);
      const bounds = visibleBounds(camera, retina);

      expect(tiles.length).toBeLessThanOrEqual(192);
      const corners: Array<[number, number]> = [
        [bounds.west, bounds.north],
        [bounds.east, bounds.north],
        [bounds.west, bounds.south],
        [bounds.east, bounds.south],
      ];
      for (const [lon, lat] of corners) {
        const covered = tiles.some((t) => {
          const b = tileBounds(t.z, t.x, t.y);
          return (
            lon >= b.west + t.lonOffset - 1e-9 &&
            lon <= b.east + t.lonOffset + 1e-9 &&
            lat <= b.north + 1e-9 &&
            lat >= b.south - 1e-9
          );
        });
        expect(covered, `corner ${lon},${lat} uncovered at pxPerDeg ${c.pxPerDeg}`).toBe(true);
      }
    }
  });

  it("respects the budget", () => {
    const c = clampCamera({ centerLon: 0, centerLat: 0, pxPerDeg: MAX_PX_PER_DEG }, view);
    expect(visibleTiles(c, view, 16).length).toBeLessThanOrEqual(16);
  });
});

describe("tileBounds", () => {
  it("tiles the globe with no gaps", () => {
    for (const z of [0, 1, 3]) {
      let cursor = -180;
      for (let x = 0; x < tileColumns(z); x++) {
        const b = tileBounds(z, x, 0);
        expect(b.west).toBeCloseTo(cursor, 9);
        cursor = b.east;
      }
      expect(cursor).toBeCloseTo(180, 9);
      expect(tileBounds(z, 0, 0).north).toBeCloseTo(90, 9);
      expect(tileBounds(z, 0, tileRows(z) - 1).south).toBeCloseTo(-90, 9);
    }
  });
});

describe("glyph lattice", () => {
  it("picks a step that clears the target spacing without huge gaps", () => {
    for (const px of [0.5, 2, 8, 24, 60, 200, MAX_PX_PER_DEG]) {
      const step = glyphStepDegrees(px, 40);
      const spacing = step * px;
      expect(spacing).toBeGreaterThanOrEqual(40);
      // The ladder's rungs are half a step apart at most, so a satisfying
      // step never overshoots the target by much (M33) — which is what keeps
      // a glyph sized from it from ballooning.
      expect(spacing).toBeLessThan(40 * 1.6);
    }
  });

  it("never gets sparser as you zoom in", () => {
    let previous = Infinity;
    for (let px = 0.5; px <= MAX_PX_PER_DEG; px *= 1.4) {
      const step = glyphStepDegrees(px, 40);
      expect(step).toBeLessThanOrEqual(previous);
      previous = step;
    }
  });

  /**
   * The bug this replaced: anchoring the lattice to each tile left
   * `tileWidth % spacing` of empty space at every tile edge, so glyphs appeared
   * in clusters with gaps between them. Points must be continuous across a
   * shared edge — no gap, and no duplicate.
   */
  it("is continuous across a tile boundary", () => {
    const step = 2.5;
    for (const z of [1, 3, 5]) {
      const columns = tileColumns(z);
      for (let x = 0; x < Math.min(columns - 1, 6); x++) {
        const left = glyphLattice(tileBounds(z, x, 0), step);
        const right = glyphLattice(tileBounds(z, x + 1, 0), step);
        if (left.cols === 0 || right.cols === 0) continue;

        const lastOfLeft = left.originLon + (left.cols - 1) * step;
        expect(right.originLon - lastOfLeft).toBeCloseTo(step, 9);
      }
    }
  });

  it("covers a tile exactly once", () => {
    const step = 5;
    const z = 2;
    const seen = new Set<string>();
    let total = 0;
    for (let x = 0; x < tileColumns(z); x++) {
      const lattice = glyphLattice(tileBounds(z, x, 0), step);
      for (let c = 0; c < lattice.cols; c++) {
        const lon = lattice.originLon + c * step;
        const key = lon.toFixed(6);
        expect(seen.has(key), `longitude ${key} drawn twice`).toBe(false);
        seen.add(key);
        total++;
      }
    }
    // A 5-degree lattice across 360 degrees is 72 columns, each drawn once.
    expect(total).toBe(72);
  });

  it("stays inside the bounds it was given", () => {
    const step = 1;
    const bounds = tileBounds(4, 7, 3);
    const lattice = glyphLattice(bounds, step);
    expect(lattice.originLon).toBeGreaterThanOrEqual(bounds.west - 1e-9);
    expect(lattice.originLon + (lattice.cols - 1) * step).toBeLessThan(bounds.east + 1e-9);
    expect(lattice.originLat).toBeLessThanOrEqual(bounds.north + 1e-9);
    expect(lattice.originLat - (lattice.rows - 1) * step).toBeGreaterThan(bounds.south - 1e-9);
  });
});

describe("the cylindrical projections (M11)", () => {
  const view = { width: 800, height: 400 };

  it("round-trips a screen point back to where it came from", () => {
    // `unproject` is on the *painting* path: a stroke drawn at 60°N has to
    // land at 60°N, in every projection (M11's acceptance).
    for (const projection of PROJECTIONS) {
      const camera = clampCamera(
        { centerLon: 10, centerLat: 40, pxPerDeg: 8, projection: projection.id },
        view,
      );
      for (const point of [
        { x: 100, y: 80 },
        { x: 400, y: 200 },
        { x: 750, y: 350 },
      ]) {
        const geo = unproject(camera, view, point);
        const back = project(camera, view, geo);
        expect(back.x, `${projection.id} x`).toBeCloseTo(point.x, 6);
        expect(back.y, `${projection.id} y`).toBeCloseTo(point.y, 6);
      }
    }
  });

  it("keeps the point under the cursor under the cursor while zooming", () => {
    for (const projection of PROJECTIONS) {
      const camera = clampCamera(
        { centerLon: 0, centerLat: 30, pxPerDeg: 6, projection: projection.id },
        view,
      );
      const anchor = { x: 200, y: 120 };
      const before = unproject(camera, view, anchor);
      const zoomed = zoomAbout(camera, view, anchor, 1.8);
      const after = unproject(zoomed, view, anchor);
      expect(after.lon, `${projection.id} lon`).toBeCloseTo(before.lon, 4);
      expect(after.lat, `${projection.id} lat`).toBeCloseTo(before.lat, 4);
    }
  });

  it("never scrolls the map off the top or bottom", () => {
    for (const projection of PROJECTIONS) {
      const clamped = clampCamera(
        { centerLon: 0, centerLat: 89.9, pxPerDeg: 40, projection: projection.id },
        view,
      );
      const top = project(clamped, view, { lon: 0, lat: projection.maxLat });
      expect(top.y, `${projection.id}`).toBeLessThanOrEqual(0.001);
    }
  });

  it("carries the projection through a clamp, so the camera keeps it", () => {
    // Everything downstream reads the projection off the camera it is handed;
    // a clamp that dropped the field would silently draw the world flat again.
    const clamped = clampCamera(
      { centerLon: 0, centerLat: 10, pxPerDeg: 6, projection: "mercator" },
      view,
    );
    expect(clamped.projection).toBe("mercator");
    expect(zoomAbout(clamped, view, { x: 10, y: 10 }, 2).projection).toBe("mercator");
  });

  it("culls to the band of latitude actually on screen", () => {
    // Under Mercator the same pixel height is a narrower band of latitude the
    // further north it sits. Culling to the equirectangular band instead would
    // ask for tiles that are not on screen and skip ones that are.
    const camera = { centerLon: 0, centerLat: 60, pxPerDeg: 4, projection: "mercator" as const };
    const bounds = visibleBounds(camera, view);
    const halfY = view.height / 2 / camera.pxPerDeg;
    const mercator = projectionOf("mercator");
    expect(bounds.north).toBeCloseTo(mercator.latOf(mercator.yOf(60) + halfY), 6);
    expect(bounds.south).toBeCloseTo(mercator.latOf(mercator.yOf(60) - halfY), 6);
    // And it really is narrower than the flat reading would have been.
    expect(bounds.north - bounds.south).toBeLessThan(2 * halfY);
  });

  it("keeps the grabbed point under the pointer while panning", () => {
    // A drag is a drag of the *map*: whatever was under the pointer when it
    // went down is still under it as the hand moves. Panning in degrees of
    // latitude instead makes the map run away from the pointer under Mercator,
    // further the higher the latitude.
    for (const projection of PROJECTIONS) {
      const camera = clampCamera(
        { centerLon: 0, centerLat: 20, pxPerDeg: 12, projection: projection.id },
        view,
      );
      const grabbed = { x: 300, y: 140 };
      const under = unproject(camera, view, grabbed);
      const moved = { x: grabbed.x - 70, y: grabbed.y + 45 };
      const panned = panBy(camera, view, -(moved.x - grabbed.x), -(moved.y - grabbed.y));
      const back = project(panned, view, under);
      expect(back.x, `${projection.id} x`).toBeCloseTo(moved.x, 6);
      expect(back.y, `${projection.id} y`).toBeCloseTo(moved.y, 6);
    }
  });

  it("leaves the equirectangular numbers exactly as they were", () => {
    // A camera that names no projection is the one the app grew up in, so
    // every existing caller keeps the behaviour it had.
    const camera = { centerLon: 12, centerLat: -20, pxPerDeg: 5 };
    const point = { lon: 40, lat: 33 };
    expect(project(camera, view, point)).toEqual(
      project({ ...camera, projection: "equirectangular" }, view, point),
    );
  });
});
