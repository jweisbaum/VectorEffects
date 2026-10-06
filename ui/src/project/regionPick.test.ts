import { describe, expect, it } from "vitest";

import {
  ANTARCTIC,
  ARCTIC,
  dragRegion,
  followCentre,
  isWholeEarth,
  moveRegion,
  normalizeLon,
  pickerToLonLat,
  regionContains,
  regionFromView,
  regionNodes,
  regionProblem,
  regionSpan,
  snapEdges,
} from "./regionPick";
import { estimatedGribBytes, formatRegion } from "./format";

describe("drawing a box on the picker", () => {
  it("runs rightwards from 170°E to 170°W as the 20° across the antimeridian", () => {
    // Rightwards past 180: the pointer's longitude keeps growing (190), as
    // the picker reports it unwrapped about its centre.
    const r = dragRegion({ lon: 170, lat: -10 }, { lon: 190, lat: 10 });
    expect(r).toEqual({ west: 170, east: -170, south: -10, north: 10, full_circle: false });
    expect(regionSpan(r)).toBe(20);
  });

  it("runs leftwards over the same two meridians as the 340° the other way", () => {
    const r = dragRegion({ lon: 170, lat: -10 }, { lon: -170, lat: 10 });
    expect(r.west).toBe(-170);
    expect(r.east).toBe(170);
    expect(regionSpan(r)).toBe(340);
  });

  it("covers every longitude once the drag has gone a whole turn", () => {
    const r = dragRegion({ lon: 0, lat: 40 }, { lon: 365, lat: 70 });
    expect(r.full_circle).toBe(true);
    expect(regionSpan(r)).toBe(360);
  });

  it("clamps its latitudes to the poles before anything is sent", () => {
    // Rust refuses a latitude past the pole rather than clamping it.
    const r = dragRegion({ lon: 0, lat: -95 }, { lon: 10, lat: 120 });
    expect([r.south, r.north]).toEqual([-90, 90]);
  });
});

describe("the canvas", () => {
  it("is equirectangular about its centre longitude", () => {
    expect(pickerToLonLat(180, 90, 0, 360, 180)).toEqual({ lon: 0, lat: 0 });
    expect(pickerToLonLat(0, 0, 0, 360, 180)).toEqual({ lon: -180, lat: 90 });
    expect(pickerToLonLat(360, 180, 180, 360, 180)).toEqual({ lon: 360, lat: -90 });
  });

  it("follows the drag past its edge, so a box can cross 180° without leaving it", () => {
    // Centred on Greenwich, 180° is at the very edge. Holding the pointer in
    // the right-hand margin turns the canvas east until it is well inside.
    const width = 360;
    let centre = 0;
    for (let i = 0; i < 200 && centre < 120; i += 1) centre = followCentre(centre, width - 2, width);
    expect(centre).toBeGreaterThanOrEqual(120);
    const at = pickerToLonLat(width / 2 + (180 - centre) * (width / 360), 90, centre, width, 180);
    expect(at.lon).toBeCloseTo(180);
    // In the middle of the canvas nothing moves; at the left edge it turns west.
    expect(followCentre(0, width / 2, width)).toBe(0);
    expect(followCentre(0, 1, width)).toBeLessThan(0);
  });
});

describe("snapping, as Rust does it", () => {
  it("moves each edge outward to the lattice", () => {
    const r = snapEdges({ west: 10.1, east: 19.9, south: 40.1, north: 49.9, full_circle: false }, 0.25);
    expect(r).toEqual({ west: 10, east: 20, south: 40, north: 50, full_circle: false });
  });

  it("snaps a box across the antimeridian on its eastward arc", () => {
    const r = snapEdges({ west: 160.1, east: -160.1, south: -10, north: 10, full_circle: false }, 1);
    expect(r).toEqual({ west: 160, east: -160, south: -10, north: 10, full_circle: false });
  });

  it("makes any full turn the canonical west -180", () => {
    const r = snapEdges({ west: 33, east: 33, south: 60, north: 90, full_circle: true }, 0.25);
    expect(r).toEqual(ARCTIC);
  });
});

describe("presets and the whole earth", () => {
  it("fills a cap from 60° to either pole, all the way round", () => {
    expect(ARCTIC).toEqual({ west: -180, east: 180, south: 60, north: 90, full_circle: true });
    expect(ANTARCTIC).toEqual({ west: -180, east: 180, south: -90, north: -60, full_circle: true });
    expect(regionSpan(ARCTIC)).toBe(360);
  });

  it("knows a full circle from pole to pole is a global project", () => {
    expect(isWholeEarth({ ...ARCTIC, south: -90 })).toBe(true);
    expect(isWholeEarth(ARCTIC)).toBe(false);
    expect(isWholeEarth({ west: -180, east: 179.75, south: -90, north: 90, full_circle: false })).toBe(false);
  });
});

describe("the size of what is exported", () => {
  it("counts nodes as the region's lattice does", () => {
    // 160°E–160°W, 10°S–10°N at 0.25°: 161 × 81 (region.rs).
    expect(regionNodes({ west: 160, east: -160, south: -10, north: 10, full_circle: false }, 0.25)).toEqual({ ni: 161, nj: 81 });
    // A full circle holds no duplicated column, as the global grid does not.
    expect(regionNodes(ARCTIC, 0.25)).toEqual({ ni: 1440, nj: 121 });
  });

  it("estimates the export from the region's nodes, and the globe's without one", () => {
    const box = { west: 160, east: -160, south: -10, north: 10, full_circle: false };
    expect(estimatedGribBytes(0.25, 24, box)).toBe(161 * 81 * 2 * 2 * 24);
    expect(estimatedGribBytes(0.25, 24)).toBe(1440 * 721 * 2 * 2 * 24);
  });
});

describe("moving a drawn box", () => {
  it("carries both longitudes and keeps the latitudes on the earth", () => {
    const start = { west: 170, east: -170, south: 70, north: 80, full_circle: false };
    const moved = moveRegion(start, 20, 20);
    expect(moved.west).toBe(-170);
    expect(moved.east).toBe(-150);
    expect([moved.south, moved.north]).toEqual([80, 90]);
  });
});

describe("the current view", () => {
  it("reads an unwrapped view across the antimeridian as its eastward arc", () => {
    // MapHandle.bounds(): [west, north, east, south], east unwrapped.
    expect(regionFromView([150, 20, 210, -20])).toEqual({ west: 150, east: -150, south: -20, north: 20, full_circle: false });
  });

  it("covers every longitude when the view does", () => {
    expect(regionFromView([-200, 90, 200, 50]).full_circle).toBe(true);
  });
});

describe("the region as the status bar shows it", () => {
  it("names hemispheres rather than signs", () => {
    // ProjectSummary.region: east unwrapped.
    expect(formatRegion({ west: 160, east: 200, south: -10, north: 10, full_circle: false }))
      .toEqual({ lon: "160°E – 160°W", lat: "10°S – 10°N" });
    expect(formatRegion({ west: -5.25, east: 0, south: 0, north: 50.5, full_circle: false }))
      .toEqual({ lon: "5.25°W – 0°", lat: "0° – 50.5°N" });
    expect(formatRegion({ west: -180, east: 180, south: 60, north: 90, full_circle: true }).lon).toBeNull();
  });
});

describe("regionContains on the lattice", () => {
  it("holds a meridian a few ULP west of the west edge", () => {
    const lons = Array.from({ length: 3600 }, (_, i) => {
      let lon = 0 + i * 0.1;
      while (lon >= 180) lon -= 360;
      return lon;
    });
    for (let k = 0; k < 3600; k++) {
      const west = Math.round(-1800 + k) / 10;
      const req = { west, east: normalizeLon(west + 1), south: 0, north: 10, full_circle: false };
      const held = lons.filter((lon) => regionContains(req, lon, 5)).length;
      expect(held, `west ${west}`).toBe(11);
    }
  });
});

describe("regionProblem", () => {
  it("names what makes a box no region at all", () => {
    expect(regionProblem({ west: 10, east: 20, south: 40, north: 50, full_circle: false }, 0.25)).toBeNull();
    expect(regionProblem({ west: 10, east: 20, south: 50, north: 40, full_circle: false }, 0.25)).toBe("no-height");
    expect(regionProblem({ west: 10, east: 20, south: 40, north: 40, full_circle: false }, 0.25)).toBe("no-height");
    expect(regionProblem({ west: 10, east: 10, south: 40, north: 50, full_circle: false }, 0.25)).toBe("no-width");
    // A full circle has width whatever east says, and the whole earth is Global.
    expect(regionProblem({ ...ARCTIC, east: -180 }, 0.25)).toBeNull();
    expect(regionProblem({ ...ARCTIC, south: -90 }, 0.25)).toBe("whole-earth");
  });

  it("counts no nodes, never a negative number, for a box that is not one", () => {
    expect(regionNodes({ west: 10, east: 20, south: 50, north: 40, full_circle: false }, 0.25)).toEqual({ ni: 0, nj: 0 });
    expect(regionNodes({ west: 10, east: 10, south: 40, north: 50, full_circle: false }, 0.25)).toEqual({ ni: 0, nj: 0 });
  });
});
