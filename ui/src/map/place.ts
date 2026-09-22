/**
 * Hit-testing an image layer on the map (spec.md 4.9, M18).
 *
 * An image's four corners come from the backend as `ImageLayerView.corners`,
 * taken from its warp — the bent quad for a warped image, the plain affine
 * one otherwise (`ve_core::warp::Warp::place`). This module projects and hit-tests
 * resize handles and asks which loaded, active image layer a point is over; the placement arithmetic stays in Rust and is
 * never duplicated here (frontend types and maths are generated, not
 * hand-written).
 *
 * Pure and free of React, so the arithmetic can be tested rather than
 * eyeballed against a chart.
 */

import type { ImageLayerView } from "../generated/ImageLayerView";
import { type Camera, type ScreenPoint, type Viewport, project } from "./camera";
import { pictureToMap } from "./align";

/** Same order as resize_image: corners clockwise, then sides clockwise. */
export function imageResizeHandles(image: ImageLayerView, camera: Camera, viewport: Viewport) {
  const pixels = [[0, 0], [1, 0], [1, 1], [0, 1], [0.5, 0], [1, 0.5], [0.5, 1], [0, 0.5]];
  const points = pixels.map(([u, v]) => project(camera, viewport,
    pictureToMap(image, u! * image.width, v! * image.height)));
  return points.map((point, index) => {
    const opposite = points[index < 4 ? (index + 2) % 4 : 4 + (index - 4 + 2) % 4]!;
    const angle = Math.atan2(point.y - opposite.y, point.x - opposite.x);
    const direction = Number.isFinite(angle) ? ((Math.round(angle / (Math.PI / 4)) % 4) + 4) % 4
      : [1, 3, 1, 3, 2, 0, 2, 0][index]!;
    return { index, point, cursor: ["ew-resize", "nwse-resize", "ns-resize", "nesw-resize"][direction]! };
  }).filter(({ point }) => Number.isFinite(point.x) && Number.isFinite(point.y));
}

/** Corners win when handles overlap on a small image. Distances are device pixels. */
export function imageResizeHandleAt(image: ImageLayerView, camera: Camera, viewport: Viewport,
  at: ScreenPoint, radius: number) {
  if (!image.loaded) return null;
  return imageResizeHandles(image, camera, viewport).find(({ point }) =>
    Math.hypot(at.x - point.x, at.y - point.y) <= radius) ?? null;
}

/**
 * Whether a screen point is inside an image's quad.
 *
 * The crossing test over the four projected corners, which is the shape the
 * outline draws — so what can be grabbed is exactly what can be seen. The
 * projection takes each corner to the copy of the world nearest the camera,
 * so an image across the antimeridian is one quad here rather than two.
 */
export function insideImage(
  image: ImageLayerView,
  camera: Camera,
  viewport: Viewport,
  at: ScreenPoint,
): boolean {
  const quad = image.corners.map((corner) =>
    project(camera, viewport, { lon: corner[0], lat: corner[1] }),
  );
  if (quad.length < 3) return false;
  let inside = false;
  for (let i = 0, j = quad.length - 1; i < quad.length; j = i++) {
    const a = quad[i] as ScreenPoint;
    const b = quad[j] as ScreenPoint;
    if (a.y > at.y !== b.y > at.y) {
      const t = (at.y - a.y) / (b.y - a.y);
      if (at.x < a.x + t * (b.x - a.x)) inside = !inside;
    }
  }
  return inside;
}

/**
 * The image a click or drag would act on: the active layer's, loaded, under
 * the pointer.
 *
 * Only the active one, for the reason its outline is only the active one's
 * (M36): a project with several charts stacked would otherwise offer
 * whichever happened to be on top, and there would be no way to say which
 * was meant.
 */
export function imageUnder(
  views: readonly ImageLayerView[],
  activeLayer: number | null,
  camera: Camera,
  viewport: Viewport,
  at: ScreenPoint,
): ImageLayerView | null {
  const image = views.find((candidate) => candidate.layer === activeLayer);
  if (!image || !image.loaded) return null;
  return insideImage(image, camera, viewport, at) ? image : null;
}
