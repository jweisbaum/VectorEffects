import { describe, expect, it } from "vitest";

import type { ImageLayerView } from "../generated/ImageLayerView";
import { project, type Camera, type Viewport } from "./camera";
import {
  cornerUnder,
  cornersOf,
  draggedCorners,
  hasArea,
  imageUnder,
  insideImage,
  movedCorners,
} from "./place";

const view: Viewport = { width: 800, height: 600 };
const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 8 };

/** An image placed north-up over a known rectangle: 20° across, 10° down. */
function image(overrides: Partial<ImageLayerView> = {}): ImageLayerView {
  return {
    layer: 7,
    path: "/tmp/chart.tif",
    loaded: true,
    width: 200,
    height: 100,
    placement: [0.1, 0, -10, 0, -0.1, 5],
    opacity: 1,
    corners: [
      [-10, 5],
      [10, 5],
      [10, -5],
      [-10, -5],
    ],
    georeferenced: true,
    ...overrides,
  };
}

describe("cornersOf", () => {
  it("takes the three that can be dragged, in the order the backend wants them", () => {
    // The backend sends four corners clockwise and takes three back; sending
    // them in the wrong order would place the image mirrored or rotated.
    const corners = cornersOf(image());
    expect(corners.topLeft).toEqual([-10, 5]);
    expect(corners.topRight).toEqual([10, 5]);
    expect(corners.bottomLeft).toEqual([-10, -5]);
  });
});

describe("cornerUnder", () => {
  it("finds a control point of the active layer only", () => {
    const views = [image()];
    const at = project(camera, view, { lon: -10, lat: 5 });
    expect(cornerUnder(views, 7, camera, view, at, 9)).toEqual({ layer: 7, corner: 0 });
    // Not the active layer: a project with several charts under it would
    // otherwise have handles from all of them on the same corner.
    expect(cornerUnder(views, 3, camera, view, at, 9)).toBeNull();
    expect(cornerUnder(views, null, camera, view, at, 9)).toBeNull();
  });

  it("finds nothing away from every corner, and nothing in a missing file", () => {
    const views = [image()];
    const away = project(camera, view, { lon: 0, lat: 0 });
    expect(cornerUnder(views, 7, camera, view, away, 9)).toBeNull();
    expect(
      cornerUnder([image({ loaded: false })], 7, camera, view, { x: 0, y: 0 }, 1e6),
    ).toBeNull();
  });
});

describe("draggedCorners", () => {
  it("moves one point and leaves the others, which is the affine", () => {
    const moved = draggedCorners(image(), 1, [30, 20], false);
    expect(moved.topLeft).toEqual([-10, 5]);
    expect(moved.topRight).toEqual([30, 20]);
    expect(moved.bottomLeft).toEqual([-10, -5]);
  });

  it("translates the whole image when the top-left is dragged north-up", () => {
    // Shift on the origin is a move: every corner follows, so the image keeps
    // its size and its shape.
    const moved = draggedCorners(image(), 0, [0, 15], true);
    expect(moved.topLeft).toEqual([0, 15]);
    expect(moved.topRight).toEqual([20, 15]);
    expect(moved.bottomLeft).toEqual([0, 5]);
  });

  it("scales about the top-left and keeps the aspect, from either handle", () => {
    // The image is twice as wide as it is tall. Dragging the right-hand handle
    // to double the width must double the height too, or a chart comes out
    // stretched — which is exactly what a hand placement is trying to avoid.
    const wider = draggedCorners(image(), 1, [30, 999], true);
    expect(wider.topRight).toEqual([30, 5]);
    expect(wider.bottomLeft[0]).toBeCloseTo(-10, 9);
    expect(wider.bottomLeft[1]).toBeCloseTo(-15, 9);

    const taller = draggedCorners(image(), 2, [999, -15], true);
    expect(taller.bottomLeft).toEqual([-10, -15]);
    expect(taller.topRight[0]).toBeCloseTo(30, 9);
    expect(taller.topRight[1]).toBeCloseTo(5, 9);
  });

  it("keeps a stretch the image already has", () => {
    // A shift-drag scales; it does not quietly correct a placement someone
    // chose. The image below is drawn square from a 2:1 picture.
    const square = image({
      corners: [
        [0, 10],
        [10, 10],
        [10, 0],
        [0, 0],
      ],
    });
    const scaled = draggedCorners(square, 1, [20, 0], true);
    expect(scaled.topRight).toEqual([20, 10]);
    expect(scaled.bottomLeft[1]).toBeCloseTo(-10, 9);
  });
});

describe("hasArea", () => {
  it("rejects three points on a line", () => {
    // The backend refuses these; asking first means a drag that would be
    // refused simply does not move, instead of filling the log at pointer rate.
    expect(
      hasArea({ topLeft: [0, 0], topRight: [10, 0], bottomLeft: [20, 0] }),
    ).toBe(false);
    expect(
      hasArea({ topLeft: [0, 0], topRight: [10, 0], bottomLeft: [0, 10] }),
    ).toBe(true);
  });

  it("measures across the antimeridian the short way", () => {
    // An image straddling 180° has corners at 170 and -170, which is 20
    // degrees apart and not 340.
    expect(
      hasArea({ topLeft: [170, 10], topRight: [-170, 10], bottomLeft: [170, 0] }),
    ).toBe(true);
  });
});

describe("insideImage", () => {
  /** The pointer has to be over the picture the outline draws, not its bounding box. */
  it("is the quad the outline draws", () => {
    const at = (lon: number, lat: number) => project(camera, view, { lon, lat });
    expect(insideImage(image(), camera, view, at(0, 0))).toBe(true);
    expect(insideImage(image(), camera, view, at(-9.5, 4.5))).toBe(true);
    expect(insideImage(image(), camera, view, at(11, 0))).toBe(false);
    expect(insideImage(image(), camera, view, at(0, 6))).toBe(false);
  });

  /** A sheared image is a quad, so its corner triangles are outside it. */
  it("follows a shear rather than a bounding box", () => {
    const sheared = image({
      corners: [
        [-10, 5],
        [10, 15],
        [10, 5],
        [-10, -5],
      ],
    });
    const at = (lon: number, lat: number) => project(camera, view, { lon, lat });
    expect(insideImage(sheared, camera, view, at(0, 5))).toBe(true);
    // Inside the bounding box, above the sheared top edge.
    expect(insideImage(sheared, camera, view, at(-8, 12))).toBe(false);
  });
});

describe("imageUnder", () => {
  /**
   * Only the active layer's picture moves (M36), for the reason only its
   * control points are drawn: several charts stacked would otherwise move
   * whichever was on top, with no way to say which was meant.
   */
  it("offers the active layer's picture and no other", () => {
    const at = project(camera, view, { lon: 0, lat: 0 });
    const other = image({ layer: 9 });
    expect(imageUnder([image(), other], 7, camera, view, at)?.layer).toBe(7);
    expect(imageUnder([image(), other], 9, camera, view, at)?.layer).toBe(9);
    expect(imageUnder([image(), other], null, camera, view, at)).toBeNull();
  });

  it("offers nothing for an image that has not loaded", () => {
    const at = project(camera, view, { lon: 0, lat: 0 });
    expect(imageUnder([image({ loaded: false })], 7, camera, view, at)).toBeNull();
  });
});

describe("movedCorners", () => {
  /** A move keeps the shape: every point takes the same delta. */
  it("carries all three points by the same delta", () => {
    const moved = movedCorners(cornersOf(image()), 5, -2);
    expect(moved.topLeft).toEqual([-5, 3]);
    expect(moved.topRight).toEqual([15, 3]);
    expect(moved.bottomLeft).toEqual([-5, -7]);
  });

  /**
   * The picture stops at the pole rather than folding over it — and the
   * whole delta is held back, not each corner, so the shape stays rigid.
   */
  it("holds the whole move back at the pole", () => {
    const moved = movedCorners(cornersOf(image()), 0, 100);
    expect(moved.topLeft[1]).toBe(90);
    // The image is 10 degrees deep, so its bottom follows to 80.
    expect(moved.bottomLeft[1]).toBe(80);
    expect(moved.topRight[1]).toBe(90);
  });

  /** Longitude wraps: a chart dragged past the dateline stays one chart. */
  it("normalises longitude", () => {
    const moved = movedCorners(cornersOf(image()), 175, 0);
    expect(moved.topRight[0]).toBeCloseTo(-175, 9);
  });
});
