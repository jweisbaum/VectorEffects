import { describe, expect, it } from "vitest";

import type { MeasurementView } from "../generated/MeasurementView";
import { arrowHead, handleUnder, pointsNeeded, projectPath } from "./measure";
import { project, type Camera, type Viewport } from "./camera";

const view: Viewport = { width: 1000, height: 600 };

describe("projectPath", () => {
  it("stays continuous across the dateline", () => {
    // A great circle from Japan to Alaska crosses 180°, so consecutive
    // vertices sit either side of it. Projecting each one on its own takes
    // them to opposite edges of the map and strokes a line straight back
    // across the world — the artefact this function exists to prevent.
    const camera: Camera = { centerLon: 180, centerLat: 50, pxPerDeg: 4 };
    const path: [number, number][] = [
      [170, 40],
      [175, 45],
      [180, 48],
      [-175, 50],
      [-170, 51],
    ];
    const screen = projectPath(camera, view, path);
    expect(screen).toHaveLength(5);

    // Every step is the same five degrees, so every step is the same width.
    const steps = screen.slice(1).map((point, i) => point.x - screen[i]!.x);
    for (const step of steps) {
      expect(step).toBeCloseTo(5 * camera.pxPerDeg, 9);
    }
  });

  it("agrees with the plain projection where there is nothing to unwrap", () => {
    const camera: Camera = { centerLon: 10, centerLat: 20, pxPerDeg: 6 };
    const path: [number, number][] = [
      [0, 0],
      [10, 20],
      [25, -15],
    ];
    const screen = projectPath(camera, view, path);
    for (const [i, point] of screen.entries()) {
      const plain = project(camera, view, { lon: path[i]![0], lat: path[i]![1] });
      expect(point.x).toBeCloseTo(plain.x, 9);
      expect(point.y).toBeCloseTo(plain.y, 9);
    }
  });

  it("carries the vertical projection, so a path bends with the map", () => {
    // The y of each vertex comes from the camera's projection. Under Mercator
    // an evenly spaced run of latitudes must not be evenly spaced on screen,
    // or the measurement is drawn somewhere the field is not (M11).
    const path: [number, number][] = [
      [0, 0],
      [0, 20],
      [0, 40],
      [0, 60],
    ];
    const flat = projectPath({ centerLon: 0, centerLat: 30, pxPerDeg: 4 }, view, path);
    const mercator = projectPath(
      { centerLon: 0, centerLat: 30, pxPerDeg: 4, projection: "mercator" },
      view,
      path,
    );
    const spacing = (points: { y: number }[]) =>
      points.slice(1).map((point, i) => Math.abs(point.y - points[i]!.y));
    const flatSteps = spacing(flat);
    expect(flatSteps[0]).toBeCloseTo(flatSteps[2]!, 9);
    const mercatorSteps = spacing(mercator);
    expect(mercatorSteps[2]).toBeGreaterThan(mercatorSteps[0]! * 1.5);
  });

  it("has nothing to say about an empty path", () => {
    expect(projectPath({ centerLon: 0, centerLat: 0, pxPerDeg: 4 }, view, [])).toEqual([]);
  });

  it("projects globe paths directly and breaks at the hidden hemisphere", () => {
    const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 4, projection: "orthographic" };
    const path = projectPath(camera, view, [[0, 0], [30, 0], [180, 0], [150, 0]]);
    expect(path[0]!.x).toBeCloseTo(project(camera, view, { lon: 0, lat: 0 }).x, 9);
    expect(path[1]!.x).toBeCloseTo(project(camera, view, { lon: 30, lat: 0 }).x, 9);
    expect(Number.isNaN(path[2]!.x)).toBe(true);
    expect(Number.isNaN(path[3]!.x)).toBe(true);
  });
});

/** A measurement with handles and nothing else, which is all the hit test reads. */
function measurement(id: number, handles: [number, number][]): MeasurementView {
  return { id, kind: "dividers", handles, paths: [], total: null, total_at: null };
}

describe("handleUnder", () => {
  const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 10 };

  it("finds the handle the pointer is on, and nothing further away", () => {
    const views = [measurement(1, [[0, 0], [10, 0]])];
    const centre = project(camera, view, { lon: 0, lat: 0 });
    expect(handleUnder(views, camera, view, centre, 8)).toEqual({ id: 1, index: 0 });
    expect(
      handleUnder(views, camera, view, { x: centre.x + 20, y: centre.y }, 8),
    ).toBeNull();
    const second = project(camera, view, { lon: 10, lat: 0 });
    expect(handleUnder(views, camera, view, second, 8)).toEqual({ id: 1, index: 1 });
  });

  it("gives the most recently placed measurement the point they share", () => {
    // Later measurements are drawn on top, so grabbing has to agree with what
    // the user can see — otherwise a handle under another one is unreachable.
    const views = [measurement(1, [[0, 0]]), measurement(2, [[0, 0]])];
    const centre = project(camera, view, { lon: 0, lat: 0 });
    expect(handleUnder(views, camera, view, centre, 8)).toEqual({ id: 2, index: 0 });
  });

  it("finds nothing when there is nothing", () => {
    expect(handleUnder([], camera, view, { x: 1, y: 1 }, 8)).toBeNull();
  });
});

describe("pointsNeeded", () => {
  it("says how many clicks make a measurement", () => {
    // A chain and a passage both need two; a ring set needs only its centre,
    // because its size comes from the option bar rather than from the map.
    expect(pointsNeeded("dividers")).toBe(2);
    expect(pointsNeeded("passage")).toBe(2);
    expect(pointsNeeded("rings")).toBe(1);
    // A feature's speed is two marks; the time between them is not a point.
    expect(pointsNeeded("motion")).toBe(2);
  });
});

describe("arrowHead", () => {
  it("points along the path, short of the handle at its end", () => {
    // A path running straight right: the tip is on the line, 5.5 px before
    // the end where the handle sits, and the barbs are 10 px further back,
    // 4.5 px either side.
    const head = arrowHead([{ x: 0, y: 50 }, { x: 100, y: 50 }], 1);
    expect(head).not.toBeNull();
    const [tip, left, right] = head!;
    expect(tip).toEqual({ x: 94.5, y: 50 });
    expect(left.x).toBeCloseTo(84.5, 9);
    expect(right.x).toBeCloseTo(84.5, 9);
    expect(Math.abs(left.y - 50)).toBeCloseTo(4.5, 9);
    expect(left.y + right.y).toBeCloseTo(100, 9);
  });

  it("takes its direction from where the path arrives, not from its chord", () => {
    // Up, then right: the head lies along the last stretch.
    const head = arrowHead([{ x: 0, y: 100 }, { x: 0, y: 0 }, { x: 60, y: 0 }], 1);
    expect(head![0]).toEqual({ x: 54.5, y: 0 });
    expect(head![1].x).toBeCloseTo(44.5, 9);
  });

  it("looks further back when the last vertices are too close to aim by", () => {
    // A densified great circle ends in vertices a pixel apart.
    const head = arrowHead([{ x: 0, y: 0 }, { x: 99, y: 0 }, { x: 100, y: 0 }], 1);
    expect(head![0].x).toBeCloseTo(94.5, 9);
    expect(head![0].y).toBeCloseTo(0, 9);
  });

  it("draws nothing on a path too short to carry one, or one that ends off the globe", () => {
    expect(arrowHead([{ x: 0, y: 0 }, { x: 8, y: 0 }], 1)).toBeNull();
    expect(arrowHead([{ x: 0, y: 0 }], 1)).toBeNull();
    expect(arrowHead([{ x: 0, y: 0 }, { x: NaN, y: NaN }], 1)).toBeNull();
  });
});

it("translates the word of a measurement's total and keeps its distance", async () => {
  const { setLanguage } = await import("../i18n");
  const { totalLabel } = await import("./measure");
  expect(totalLabel("total 12.3 nm")).toBe("total 12.3 nm");
  setLanguage("de");
  expect(totalLabel("outer 50 nm")).toBe("außen 50 nm");
  expect(totalLabel("total 12.3 nm")).toBe("gesamt 12.3 nm");
  expect(totalLabel("GC 12 nm")).toBe("GC 12 nm");
  setLanguage("en");
});
