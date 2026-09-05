/**
 * Placing an image by hand (spec.md 4.9, M18).
 *
 * An image that carries no georeference lands on the view and is dragged into
 * place by three control points — its top-left, top-right and bottom-left
 * corners. Three points determine an affine exactly, which is why there are
 * three and not four: the fourth corner follows, and a fourth handle would let
 * the user ask for a shape no affine can make.
 *
 * A plain drag moves one point and shears the image, which is the three-point
 * case. **Shift** keeps the image north-up and its aspect true, which is the
 * two-point case — translate and scale, and the one most placements want.
 *
 * Pure and free of React, so the arithmetic can be tested rather than
 * eyeballed against a chart.
 */

import type { ImageLayerView } from "../generated/ImageLayerView";
import { type Camera, type ScreenPoint, type Viewport, normalizeLon, project } from "./camera";

/** Which control point: the image's top-left, top-right or bottom-left. */
export type CornerIndex = 0 | 1 | 2;

/** The three control points, as `[lon, lat]`. */
export interface Corners {
  topLeft: [number, number];
  topRight: [number, number];
  bottomLeft: [number, number];
}

/** How near a control point the pointer counts as being on it, in CSS pixels. */
export const CORNER_REACH_CSS = 9;

/** A control point the pointer has found. */
export interface CornerPick {
  /** Which image layer. */
  layer: number;
  /** Which of its three control points. */
  corner: CornerIndex;
}

/**
 * The three control points of an image, from the four corners it reports.
 *
 * The backend sends all four — the fourth is what the outline needs — and the
 * three that can be dragged are the first, second and last of them.
 */
export function cornersOf(view: ImageLayerView): Corners {
  const at = (i: number): [number, number] => {
    const point = view.corners[i];
    return point ? [point[0], point[1]] : [0, 0];
  };
  return { topLeft: at(0), topRight: at(1), bottomLeft: at(3) };
}

/**
 * The control point nearest `at`, within `reachPx`.
 *
 * Only the active image layer's: a project with several charts under it would
 * otherwise have handles from all of them stacked on the same corner of the
 * map, and no way to say which one a drag meant.
 */
export function cornerUnder(
  views: readonly ImageLayerView[],
  activeLayer: number | null,
  camera: Camera,
  view: Viewport,
  at: ScreenPoint,
  reachPx: number,
): CornerPick | null {
  const image = views.find((candidate) => candidate.layer === activeLayer);
  if (!image || !image.loaded) return null;
  const corners = cornersOf(image);
  const points: [CornerIndex, [number, number]][] = [
    [0, corners.topLeft],
    [1, corners.topRight],
    [2, corners.bottomLeft],
  ];

  let best: CornerPick | null = null;
  let bestDistance = reachPx;
  for (const [corner, position] of points) {
    const screen = project(camera, view, { lon: position[0], lat: position[1] });
    const distance = Math.hypot(screen.x - at.x, screen.y - at.y);
    if (distance <= bestDistance) {
      bestDistance = distance;
      best = { layer: image.layer, corner };
    }
  }
  return best;
}

/**
 * Where the three control points go when one of them is dragged to `to`.
 *
 * Plain: the dragged point moves and the other two stay, which is the affine.
 *
 * With `northUp`: the image keeps its axes square to the map and its aspect
 * true. Dragging the top-left translates the whole thing; dragging either of
 * the others scales it about the top-left, taking the size from whichever
 * axis that handle owns. That is the two-point model, and it is what a chart
 * scan almost always wants — a chart is north-up, and shearing one is a way to
 * make it wrong.
 */
export function draggedCorners(
  view: ImageLayerView,
  corner: CornerIndex,
  to: [number, number],
  northUp: boolean,
): Corners {
  const corners = cornersOf(view);
  if (!northUp) {
    switch (corner) {
      case 0:
        return { ...corners, topLeft: to };
      case 1:
        return { ...corners, topRight: to };
      case 2:
        return { ...corners, bottomLeft: to };
    }
  }

  // The aspect the image is *drawn* at, not its pixel aspect: an image already
  // placed with a stretch keeps that stretch, because a shift-drag is asked to
  // scale it, not to correct it.
  const width = normalizeLon(corners.topRight[0] - corners.topLeft[0]);
  const height = corners.bottomLeft[1] - corners.topLeft[1];
  const ratio = width === 0 ? 1 : height / width;

  switch (corner) {
    case 0: {
      // Translate: the whole image follows the handle.
      const dLon = normalizeLon(to[0] - corners.topLeft[0]);
      const dLat = to[1] - corners.topLeft[1];
      return {
        topLeft: to,
        topRight: [corners.topRight[0] + dLon, corners.topRight[1] + dLat],
        bottomLeft: [corners.bottomLeft[0] + dLon, corners.bottomLeft[1] + dLat],
      };
    }
    case 1: {
      // Scale from the width, keeping the top edge along a parallel.
      const span = normalizeLon(to[0] - corners.topLeft[0]);
      return {
        topLeft: corners.topLeft,
        topRight: [corners.topLeft[0] + span, corners.topLeft[1]],
        bottomLeft: [corners.topLeft[0], corners.topLeft[1] + span * ratio],
      };
    }
    case 2: {
      // And from the height, keeping the left edge along a meridian.
      const span = to[1] - corners.topLeft[1];
      const across = ratio === 0 ? width : span / ratio;
      return {
        topLeft: corners.topLeft,
        topRight: [corners.topLeft[0] + across, corners.topLeft[1]],
        bottomLeft: [corners.topLeft[0], corners.topLeft[1] + span],
      };
    }
  }
}

/**
 * Whether three control points describe an image with any area.
 *
 * Collinear points have none, and the backend refuses them; asking first means
 * a drag that would be refused simply does not move, rather than filling the
 * log with errors at pointer rate.
 */
export function hasArea(corners: Corners): boolean {
  const ax = normalizeLon(corners.topRight[0] - corners.topLeft[0]);
  const ay = corners.topRight[1] - corners.topLeft[1];
  const bx = normalizeLon(corners.bottomLeft[0] - corners.topLeft[0]);
  const by = corners.bottomLeft[1] - corners.topLeft[1];
  return Math.abs(ax * by - ay * bx) > 1e-9;
}
