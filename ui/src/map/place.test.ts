import { describe, expect, it } from "vitest";

import type { ImageLayerView } from "../generated/ImageLayerView";
import { project, type Camera, type Viewport } from "./camera";
import { imageUnder, insideImage } from "./place";

// The corner-drag and M50 edge/rotation-grip tests that used to live here
// (cornersOf, cornerUnder, draggedCorners, hasArea, movedCorners, the edge
// and rotation grips) are deleted, not rewritten: they tested arithmetic on
// the three-control-point affine model — dragging one of three named corners
// to reshape a placement — which `Warp`'s control points replace outright.
// There is no successor gesture for them to be rewritten against yet; Task 7
// builds the alignment interaction that becomes their replacement, on top of
// `imageUnder` and `insideImage` below. The hit-testing they share is still
// covered here.

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
    control_points: [],
    warped: false,
    warp_mesh: [],
    warp_cells: 0,
    corners: [
      [-10, 5],
      [10, 5],
      [10, -5],
      [-10, -5],
    ],
    corner_residual_deg: [0, 0, 0, 0],
    georeferenced: true,
    ...overrides,
  };
}

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
   * Only the active layer's picture is offered (M36), for the reason only
   * its outline is drawn: several charts stacked would otherwise offer
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
