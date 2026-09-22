import { describe, expect, it } from "vitest";

import type { ImageLayerView } from "../generated/ImageLayerView";
import { project, type Camera, type Viewport } from "./camera";
import { imageUnder, insideImage, imageResizeHandles, imageResizeHandleAt } from "./place";

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


describe("image resize handles", () => {
  it("places corners and side midpoints on the picture, with directional cursors", () => {
    const handles = imageResizeHandles(image(), camera, view);
    expect(handles.map(h => [h.point.x, h.point.y])).toEqual([
      [320, 260], [480, 260], [480, 340], [320, 340],
      [400, 260], [480, 300], [400, 340], [320, 300],
    ]);
    expect(handles.slice(4).map(h => h.cursor)).toEqual(["ns-resize", "ew-resize", "ns-resize", "ew-resize"]);
    for (const handle of handles) {
      expect(imageResizeHandleAt(image(), camera, view, { x: handle.point.x + 6, y: handle.point.y }, 9)?.index).toBe(handle.index);
    }
    expect(imageResizeHandleAt(image(), camera, view, { x: 400, y: 300 }, 9)).toBeNull();
    expect(imageResizeHandleAt(image({ loaded: false }), camera, view, handles[0]!.point, 9)).toBeNull();
  });

  it("follows the bowed edge mesh and scales hit tolerance with display density", () => {
    const bent = image({ warped: true, warp_cells: 2, warp_mesh: [
      -10, 5, 0, 8, 10, 5, -10, 0, 0, 0, 10, 0, -10, -5, 0, -5, 10, -5,
    ] });
    const top = imageResizeHandles(bent, camera, view)[4]!;
    expect(top.point).toEqual({ x: 400, y: 236 });
    expect(imageResizeHandleAt(bent, camera, view, { x: 400, y: 220 }, 18)?.index).toBe(4);
    expect(imageResizeHandleAt(bent, camera, view, { x: 400, y: 220 }, 9)).toBeNull();
  });
});
