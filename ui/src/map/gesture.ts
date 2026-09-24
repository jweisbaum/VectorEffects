/**
 * The gesture state machine: what a press, a move and a release do.
 *
 * Split out of `MapView` because this is the part of a tool a user actually
 * feels, and it was the one part with no test — the component around it needs a
 * canvas, a camera and a pointer, so nothing here could be checked without all
 * three. It is pure: a gesture in progress, an event, and what to do next.
 *
 * The rules are keyed on the *gesture*, never on the tool. Two tools that draw
 * the same way behave the same way because they reach the same branch, which is
 * what makes the mask brush-like rather than merely similar (spec.md 6.1).
 */

import type { Gesture } from "../generated/Gesture";
import type { PathPoint } from "../generated/PathPoint";
import type { ToolSchema } from "../generated/ToolSchema";
import { normalizeLon } from "./camera";
import { gestureKind, type ToolState } from "./tools";

/**
 * A gesture part-way through being drawn.
 *
 * The four gestures that take longer than an instant. A `point` is not among
 * them: a click commits it, so it is never in progress.
 */
export type InProgress =
  | { kind: "stroke"; points: Array<[number, number]> }
  | { kind: "extent"; centre: [number, number]; rim: [number, number] }
  | { kind: "ring"; points: Array<[number, number]> }
  | { kind: "path"; nodes: PathPoint[] };

/**
 * Smallest drag that counts as one, in device pixels.
 *
 * The same slack the hand tool uses to tell a click from a pan. A preset is
 * dragged out from its centre, so a press and release at one point describes a
 * shape of no size — and a press with a pixel of hand tremor describes one just
 * barely bigger, which is worse: it commits an object the user cannot see and
 * did not ask for.
 */
export const MIN_DRAG_PX = 4;

/** The finished gesture an in-progress one becomes. */
export function finished(drawing: InProgress): Gesture {
  switch (drawing.kind) {
    case "stroke":
      return { kind: "stroke", points: drawing.points };
    case "extent":
      return { kind: "extent", centre: drawing.centre, rim: drawing.rim };
    case "ring":
      return { kind: "ring", points: drawing.points };
    case "path":
      return { kind: "path", nodes: drawing.nodes };
  }
}

/**
 * Whether a gesture has enough placed to be worth committing.
 *
 * `pxPerDeg` is the map scale, which is what turns the drag threshold into a
 * distance: the threshold is a number of pixels on screen, so at any zoom the
 * same hand movement means the same thing.
 */
export function isComplete(drawing: InProgress, pxPerDeg: number): boolean {
  switch (drawing.kind) {
    case "stroke":
      return drawing.points.length > 0;
    case "extent":
      return dragPixels(drawing, pxPerDeg) >= MIN_DRAG_PX;
    // A polygon needs three vertices to have an inside, and a curve two nodes
    // to have a length. Both are the backend's rule as well, so a gesture that
    // fails here would have been refused there.
    case "ring":
      return drawing.points.length >= 3;
    case "path":
      return drawing.nodes.length >= 2;
  }
}

/** How far a preset has been dragged, in device pixels. */
export function dragPixels(
  drawing: Extract<InProgress, { kind: "extent" }>,
  pxPerDeg: number,
): number {
  // Equirectangular, so both axes are the same number of pixels per degree and
  // there is no cosine: this is a distance on the map, not on the ground.
  const dLon = normalizeLon(drawing.rim[0] - drawing.centre[0]);
  const dLat = drawing.rim[1] - drawing.centre[1];
  return Math.hypot(dLon, dLat) * pxPerDeg;
}

/**
 * The latitude a gesture resolves its pixel sizes against.
 *
 * Where the gesture *began*, in every case — the point the object will be
 * anchored at. Taking it from wherever the pointer finished would mean a
 * footprint that changed size as a drag moved north, and a preview that
 * disagreed with what got painted.
 */
export function gestureLatitude(gesture: Gesture): number {
  switch (gesture.kind) {
    case "stroke":
    case "relocate":
      return gesture.points[0]?.[1] ?? 0;
    case "point":
      return gesture.at[1];
    case "extent":
      return gesture.centre[1];
    case "ring":
      return gesture.points[0]?.[1] ?? 0;
    case "path":
      return gesture.nodes[0]?.at[1] ?? 0;
  }
}

/**
 * The gesture a click at `at` would produce, for the hover indicator.
 *
 * Only the tools whose hover indicator exists reach this, so the gestures a
 * click cannot complete need no answer: a `point` is the whole gesture, and a
 * `stroke` is a one-point one, which is exactly what a click paints.
 */
export function hoverGesture(
  schema: ToolSchema,
  state: ToolState,
  at: { lon: number; lat: number },
): Gesture {
  const kind = gestureKind(schema, state.values);
  return kind === "point"
    ? { kind: "point", at: [at.lon, at.lat] }
    : { kind: "stroke", points: [[at.lon, at.lat]] };
}

/** What a pointer press does. */
export type Press =
  /** A stamp: the whole gesture, committed at once. */
  | { act: "commit"; gesture: Gesture }
  /** A gesture has begun, or grown by a point. */
  | { act: "draw"; drawing: InProgress; shapeHandles: boolean }
  /** A ring was closed by clicking its first vertex again. */
  | { act: "close" };

/**
 * A pointer press, given the gesture already in progress.
 *
 * `onFirstVertex` is whether the press landed on the ring's own first vertex,
 * which the caller measures in screen pixels — the only thing here that needs a
 * camera, and so the only thing passed in rather than computed.
 */
export function press(
  kind: Exclude<Gesture["kind"], "relocate">,
  current: InProgress | null,
  at: [number, number],
  onFirstVertex: boolean,
): Press {
  switch (kind) {
    // Click to place, no drag (spec.md 6.2). Committed on the press, which is
    // what makes it feel like a stamp rather than a very short gesture.
    case "point":
      return { act: "commit", gesture: { kind: "point", at } };

    case "stroke":
      return {
        act: "draw",
        drawing: { kind: "stroke", points: [at] },
        shapeHandles: false,
      };

    // Press, drag out from the centre, release. All three presets — square,
    // rectangle and circle — are this one gesture; which shape the drag becomes
    // is `shape_source`, read where the footprint is built, not here.
    case "extent":
      return {
        act: "draw",
        drawing: { kind: "extent", centre: at, rim: at },
        shapeHandles: false,
      };

    // Built up click by click. A click on the first vertex closes the ring,
    // which is how a polygon tool is expected to end and avoids needing the
    // keyboard for the common case.
    case "ring": {
      if (current?.kind === "ring") {
        if (current.points.length >= 3 && onFirstVertex) return { act: "close" };
        return {
          act: "draw",
          drawing: { kind: "ring", points: [...current.points, at] },
          shapeHandles: false,
        };
      }
      return { act: "draw", drawing: { kind: "ring", points: [at] }, shapeHandles: false };
    }

    // The pen: each press places a node, and holding shapes its handles.
    case "path": {
      const node: PathPoint = { at };
      const nodes = current?.kind === "path" ? [...current.nodes, node] : [node];
      return { act: "draw", drawing: { kind: "path", nodes }, shapeHandles: true };
    }
  }
}

/**
 * What a pointer release does.
 *
 * The gestures bounded by the pointer end here; the ones built up click by
 * click are held until they are closed or abandoned. That difference is the
 * whole of it, and it belongs to the gesture rather than to any tool.
 */
export function release(current: InProgress | null): "finish" | "hold" {
  if (current === null) return "finish";
  return current.kind === "stroke" || current.kind === "extent" ? "finish" : "hold";
}

/**
 * The handles a press-and-hold pulls out of the node just placed.
 *
 * Symmetric about the node, which is what makes a smooth corner rather than a
 * cusp — the standard behaviour of every path tool, and the reason a node is
 * placed on the press rather than on the release.
 */
export function shapeNode(
  drawing: Extract<InProgress, { kind: "path" }>,
  at: [number, number],
): void {
  const node = drawing.nodes[drawing.nodes.length - 1];
  if (!node) return;
  node.out_handle = at;
  node.in_handle = [
    normalizeLon(node.at[0] - normalizeLon(at[0] - node.at[0])),
    node.at[1] - (at[1] - node.at[1]),
  ];
}
