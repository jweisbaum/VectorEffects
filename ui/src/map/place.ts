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

// --- Edges and rotation (M50) -------------------------------------------------
//
// Three corners are an affine exactly, so they can express any placement a
// picture can have — but only by dragging two of them in turn. Rotating a
// chart scan meant moving the top-right and then the bottom-left and hoping
// they agreed; nudging one side in meant moving two corners by the same
// amount by eye. The edges and the rotation grip are those gestures as one
// each. Every one of them still lands as the same three points, so the
// document, the command and the undo are unchanged.

/** Which side of the picture a grip belongs to. */
export type EdgeName = "top" | "right" | "bottom" | "left";

/** What a drag has hold of. */
export type Grip =
  | { kind: "corner"; corner: CornerIndex }
  | { kind: "edge"; edge: EdgeName }
  | { kind: "rotate" };

/** A grip the pointer has found, and the layer it belongs to. */
export interface GripPick {
  layer: number;
  grip: Grip;
}

/** How far outside the top edge the rotation grip sits, as a fraction of the height. */
const ROTATE_OFFSET = 0.16;

/** The least of an axis a drag may leave, so a picture cannot be folded flat. */
const MIN_SPAN = 0.02;

type Point = [number, number];

/** The image's own axes: across the top, and down the left. */
function axes(corners: Corners): { u: Point; v: Point } {
  return {
    u: [normalizeLon(corners.topRight[0] - corners.topLeft[0]), corners.topRight[1] - corners.topLeft[1]],
    v: [normalizeLon(corners.bottomLeft[0] - corners.topLeft[0]), corners.bottomLeft[1] - corners.topLeft[1]],
  };
}

/** The corner the other three imply: the affine's fourth point. */
export function fourthCorner(corners: Corners): Point {
  const { u, v } = axes(corners);
  return [corners.topLeft[0] + u[0] + v[0], corners.topLeft[1] + u[1] + v[1]];
}

/** The middle of the picture. */
export function centreOf(corners: Corners): Point {
  const { u, v } = axes(corners);
  return [
    corners.topLeft[0] + (u[0] + v[0]) / 2,
    corners.topLeft[1] + (u[1] + v[1]) / 2,
  ];
}

/** The midpoint of each edge, where its grip sits. */
export function edgeGrips(corners: Corners): Record<EdgeName, Point> {
  const { u, v } = axes(corners);
  const at = (a: number, b: number): Point => [
    corners.topLeft[0] + a * u[0] + b * v[0],
    corners.topLeft[1] + a * u[1] + b * v[1],
  ];
  return { top: at(0.5, 0), right: at(1, 0.5), bottom: at(0.5, 1), left: at(0, 0.5) };
}

/**
 * Where the rotation grip sits: outside the top edge, on the picture's own
 * up-axis, so it stays where the eye expects it however the picture is turned.
 */
export function rotateGrip(corners: Corners): Point {
  const { u, v } = axes(corners);
  return [
    corners.topLeft[0] + u[0] / 2 - v[0] * ROTATE_OFFSET,
    corners.topLeft[1] + u[1] / 2 - v[1] * ROTATE_OFFSET,
  ];
}

/**
 * A point in the picture's own coordinates: `[0,0]` is the top-left corner,
 * `[1,1]` the fourth, whatever the picture has been sheared or turned into.
 *
 * Null when the two axes are parallel, which is a picture with no area — the
 * one case that has no answer rather than a wrong one.
 */
function imageSpace(corners: Corners, at: Point): Point | null {
  const { u, v } = axes(corners);
  const det = u[0] * v[1] - u[1] * v[0];
  if (Math.abs(det) < 1e-12) return null;
  const d: Point = [normalizeLon(at[0] - corners.topLeft[0]), at[1] - corners.topLeft[1]];
  return [(d[0] * v[1] - d[1] * v[0]) / det, (u[0] * d[1] - u[1] * d[0]) / det];
}

/**
 * Where the corners go when an edge is dragged to `to`.
 *
 * The edge follows the pointer and the opposite edge stays, which is what
 * dragging a side means everywhere else. The pointer's position is read in
 * the picture's own coordinates, so a sheared or turned picture answers the
 * gesture along its own axes rather than the map's.
 */
export function draggedEdge(corners: Corners, edge: EdgeName, to: Point): Corners {
  const at = imageSpace(corners, to);
  if (!at) return corners;
  const { u, v } = axes(corners);
  const [alpha, beta] = at;
  const shift = (point: Point, by: Point, k: number): Point => [
    point[0] + k * by[0],
    point[1] + k * by[1],
  ];
  switch (edge) {
    case "top": {
      const t = Math.min(beta, 1 - MIN_SPAN);
      return {
        topLeft: shift(corners.topLeft, v, t),
        topRight: shift(corners.topRight, v, t),
        bottomLeft: corners.bottomLeft,
      };
    }
    case "bottom": {
      const s = Math.max(beta, MIN_SPAN);
      return { ...corners, bottomLeft: shift(corners.topLeft, v, s) };
    }
    case "left": {
      const t = Math.min(alpha, 1 - MIN_SPAN);
      return {
        topLeft: shift(corners.topLeft, u, t),
        topRight: corners.topRight,
        bottomLeft: shift(corners.bottomLeft, u, t),
      };
    }
    case "right": {
      const s = Math.max(alpha, MIN_SPAN);
      return { ...corners, topRight: shift(corners.topLeft, u, s) };
    }
  }
}

/**
 * Where the corners go when the picture is turned so its top points at `to`.
 *
 * About the centre, and stateless: the grip goes where the pointer is, so
 * there is no start angle to remember and letting go and grabbing again picks
 * up exactly where it left off.
 *
 * Turned in a space where a degree of longitude is scaled by the cosine of
 * the centre's latitude. The placement is affine in *degrees* (spec.md 4.9),
 * and a rotation in raw degrees is a shear on screen everywhere but the
 * equator — the picture would lean rather than turn.
 */
export function rotatedCorners(corners: Corners, to: Point): Corners {
  const centre = centreOf(corners);
  const scale = Math.max(Math.cos((centre[1] * Math.PI) / 180), 1e-6);
  const flat = (point: Point): Point => [normalizeLon(point[0] - centre[0]) * scale, point[1] - centre[1]];
  const round = (point: Point): Point => [centre[0] + point[0] / scale, centre[1] + point[1]];

  const { v } = axes(corners);
  // The picture's own up, which is the way the grip points from the centre.
  const up: Point = [-v[0] * scale, -v[1]];
  const want = flat(to);
  if (Math.hypot(...up) < 1e-12 || Math.hypot(...want) < 1e-12) return corners;
  const angle = Math.atan2(want[1], want[0]) - Math.atan2(up[1], up[0]);
  const [cos, sin] = [Math.cos(angle), Math.sin(angle)];
  const turn = (point: Point): Point => {
    const [x, y] = flat(point);
    return round([x * cos - y * sin, x * sin + y * cos]);
  };
  return {
    topLeft: turn(corners.topLeft),
    topRight: turn(corners.topRight),
    bottomLeft: turn(corners.bottomLeft),
  };
}

/** Where the corners go when a grip is dragged to `to`. */
export function draggedByGrip(
  view: ImageLayerView,
  grip: Grip,
  to: Point,
  northUp: boolean,
): Corners {
  const corners = cornersOf(view);
  switch (grip.kind) {
    case "corner":
      return draggedCorners(view, grip.corner, to, northUp);
    case "edge":
      return draggedEdge(corners, grip.edge, to);
    case "rotate":
      return rotatedCorners(corners, to);
  }
}

/**
 * The grip nearest `at`, within `reachPx`, on the active image layer.
 *
 * Corners first, then edges, then the rotation grip: where two are within
 * reach of one another the corner is the one a hand means, since it is the
 * finer control and the edges' midpoints are never near a corner unless the
 * picture is nearly flat.
 */
export function gripUnder(
  views: readonly ImageLayerView[],
  activeLayer: number | null,
  camera: Camera,
  viewport: Viewport,
  at: ScreenPoint,
  reachPx: number,
): GripPick | null {
  const image = views.find((candidate) => candidate.layer === activeLayer);
  if (!image || !image.loaded) return null;
  const corners = cornersOf(image);
  const edges = edgeGrips(corners);
  const candidates: [Grip, Point][] = [
    [{ kind: "corner", corner: 0 }, corners.topLeft],
    [{ kind: "corner", corner: 1 }, corners.topRight],
    [{ kind: "corner", corner: 2 }, corners.bottomLeft],
    [{ kind: "edge", edge: "top" }, edges.top],
    [{ kind: "edge", edge: "right" }, edges.right],
    [{ kind: "edge", edge: "bottom" }, edges.bottom],
    [{ kind: "edge", edge: "left" }, edges.left],
    [{ kind: "rotate" }, rotateGrip(corners)],
  ];

  let best: GripPick | null = null;
  let bestDistance = reachPx;
  for (const [grip, position] of candidates) {
    const screen = project(camera, viewport, { lon: position[0], lat: position[1] });
    const distance = Math.hypot(screen.x - at.x, screen.y - at.y);
    // Strictly nearer, so the earlier entries win a tie: corners over edges.
    if (distance < bestDistance) {
      bestDistance = distance;
      best = { layer: image.layer, grip };
    }
  }
  return best;
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
 * The image a drag would move: the active layer's, loaded, under the pointer.
 *
 * Only the active one, for the reason its handles are only the active one's
 * (M36): a project with several charts stacked would otherwise move whichever
 * happened to be on top, and there would be no way to say which was meant.
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

/**
 * The three control points moved bodily by a delta in degrees (M36).
 *
 * Every point takes the same delta, so the image keeps its size, its aspect
 * and whatever shear it had: a move is not a placement. The latitude delta is
 * held back where it would carry a corner off the map — the picture stops at
 * the pole rather than folding over it — and holding the *whole* delta rather
 * than each corner is what keeps the shape rigid on the way.
 */
export function movedCorners(corners: Corners, dLon: number, dLat: number): Corners {
  const lats = [corners.topLeft[1], corners.topRight[1], corners.bottomLeft[1]];
  const room = Math.min(...lats.map((lat) => 90 - lat));
  const below = Math.min(...lats.map((lat) => lat + 90));
  const held = Math.max(-below, Math.min(dLat, room));
  const moved = (point: [number, number]): [number, number] => [
    normalizeLon(point[0] + dLon),
    point[1] + held,
  ];
  return {
    topLeft: moved(corners.topLeft),
    topRight: moved(corners.topRight),
    bottomLeft: moved(corners.bottomLeft),
  };
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
