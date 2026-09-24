/**
 * Drawing and hit-testing the measurement overlays (spec.md 10, M8).
 *
 * **Nothing here measures anything.** The distances, the bearings, the
 * polylines and the labels all arrive from Rust already computed and already
 * formatted — this module projects the geography onto the screen, strokes it,
 * and says which handle is under the pointer. That split is the same one the
 * readout follows, and it is what keeps one implementation of the geodesy in
 * the application rather than two that can disagree.
 *
 * Pure and free of React, so the geometry can be tested rather than looked at.
 */

import { mapColour } from "../settings/themes";
import type { MeasurementView } from "../generated/MeasurementView";
import type { MeasurementKind } from "../generated/MeasurementKind";
import {
  type Camera,
  type ScreenPoint,
  type Viewport,
  normalizeLon,
  project,
  projectionFor,
} from "./camera";

/** How near a handle the pointer counts as being on it, in CSS pixels. */
export const HANDLE_REACH_CSS = 8;

/** A handle the pointer has found. */
export interface HandlePick {
  /** Which measurement it belongs to. */
  id: number;
  /** Which of that measurement's placed points it is. */
  index: number;
}

/**
 * Projects a geographic polyline to a continuous screen path.
 *
 * `project` takes each point to the copy of the world nearest the camera, which
 * is right for one point and wrong for a line: a great circle crossing the
 * dateline would have consecutive vertices a world apart and would be stroked
 * straight back across the map. So the first vertex is placed normally and
 * every one after it follows its neighbour by the *short* way round, which
 * keeps the line continuous however far it runs in longitude.
 *
 * A path that has come more than half a world from where it started therefore
 * leaves the viewport rather than wrapping into it, which is what the world
 * copies in {@link drawMeasurements} are for.
 */
export function projectPath(
  camera: Camera,
  view: Viewport,
  points: ReadonlyArray<readonly [number, number]>,
): ScreenPoint[] {
  const first = points[0];
  if (first === undefined) return [];
  // Two-dimensional projections do not have a linear longitude axis. Project
  // each vertex directly; NaN vertices mark the hidden hemisphere and are
  // consumed by the drawing loop below as breaks in the visible path.
  if (projectionFor(camera).general) {
    return points.map(([lon, lat]) => project(camera, view, { lon, lat }));
  }
  const origin = project(camera, view, { lon: first[0], lat: first[1] });
  const out: ScreenPoint[] = [origin];

  // Degrees travelled in longitude since the first vertex, accumulated the
  // short way at each step. `project` would normalise this back, which is why
  // x is computed here and only y is taken from it — y depends on the latitude
  // and the projection, and on nothing this loop is unwrapping.
  let travelled = 0;
  let previousLon = first[0];
  for (const [lon, lat] of points.slice(1)) {
    travelled += normalizeLon(lon - previousLon);
    previousLon = lon;
    out.push({
      x: origin.x + travelled * camera.pxPerDeg,
      y: project(camera, view, { lon, lat }).y,
    });
  }
  return out;
}

/**
 * The handle nearest `at`, within `reachPx`.
 *
 * Later measurements win ties, so the one most recently placed — which is the
 * one drawn on top — is the one grabbed.
 */
export function handleUnder(
  views: readonly MeasurementView[],
  camera: Camera,
  view: Viewport,
  at: ScreenPoint,
  reachPx: number,
): HandlePick | null {
  let best: HandlePick | null = null;
  let bestDistance = reachPx;
  for (const measurement of views) {
    for (const [index, handle] of measurement.handles.entries()) {
      const point = project(camera, view, { lon: handle[0]!, lat: handle[1]! });
      const distance = Math.hypot(point.x - at.x, point.y - at.y);
      if (distance <= bestDistance) {
        bestDistance = distance;
        best = { id: measurement.id, index };
      }
    }
  }
  return best;
}

/** What to draw, beyond the measurements themselves. */
export interface MeasureOverlay {
  /** The measurements, as the backend computed them. */
  views: readonly MeasurementView[];
  /** A point placed but not yet joined to a second one. */
  pending?: readonly [number, number] | null;
  /** Where the pointer is, for the rubber line out of `pending`. */
  cursor?: ScreenPoint | null;
  /** A measurement whose handles are drawn filled, because it is being edited. */
  active?: number | null;
}

/**
 * Draws every measurement, in device pixels.
 *
 * Each path is stroked three times, at longitude offsets of -360, 0 and +360,
 * for the same reason the map draws the world three times: a passage across
 * the Pacific belongs to whichever copy the camera is looking at, and one copy
 * would be missing from the other.
 */
export function drawMeasurements(
  context: CanvasRenderingContext2D,
  camera: Camera,
  view: Viewport,
  dpr: number,
  overlay: MeasureOverlay,
): void {
  const worldPx = 360 * camera.pxPerDeg;
  const copies = projectionFor(camera).general ? [0] : [-worldPx, 0, worldPx];

  context.save();
  context.lineJoin = "round";
  context.lineCap = "round";
  context.font = `${11 * dpr}px system-ui, sans-serif`;

  for (const measurement of overlay.views) {
    for (const path of measurement.paths) {
      const screen = projectPath(camera, view, path.points as [number, number][]);
      if (screen.length < 2) continue;
      // The rhumb line is dashed and the great circle solid, so a reader
      // looking at two curves between the same two points can tell which is
      // the one they would steer (spec.md 10).
      const dashed = path.kind === "rhumb";
        for (const offset of copies) {
        context.beginPath();
        let drawing = false;
        for (const point of screen) {
          if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) {
            drawing = false;
            continue;
          }
          if (drawing) context.lineTo(point.x + offset, point.y);
          else context.moveTo(point.x + offset, point.y);
          drawing = true;
        }
        context.strokeStyle = mapColour("measure", dashed ? 0.75 : 0.95);
        context.lineWidth = Math.max(1, dpr) * (dashed ? 1.4 : 1.8);
        context.setLineDash(dashed ? [7 * dpr, 5 * dpr] : []);
        context.stroke();
      }
      context.setLineDash([]);

      const label = project(camera, view, {
        lon: path.label_at[0]!,
        lat: path.label_at[1]!,
      });
      if (Number.isFinite(label.x) && Number.isFinite(label.y)) {
        writeLabel(context, dpr, path.label, label, copies);
      }
    }

    if (measurement.total !== null && measurement.total_at !== null) {
      const at = project(camera, view, {
        lon: measurement.total_at[0]!,
        lat: measurement.total_at[1]!,
      });
      if (Number.isFinite(at.x) && Number.isFinite(at.y)) {
        writeLabel(context, dpr, measurement.total, { x: at.x, y: at.y + 16 * dpr }, copies, true);
      }
    }

    const active = overlay.active === measurement.id;
    for (const handle of measurement.handles) {
      const at = project(camera, view, { lon: handle[0]!, lat: handle[1]! });
      if (!Number.isFinite(at.x) || !Number.isFinite(at.y)) continue;
      for (const offset of copies) {
        context.beginPath();
        context.arc(at.x + offset, at.y, 4.5 * dpr, 0, Math.PI * 2);
        context.fillStyle = active
          ? mapColour("measure", 0.95)
          : mapColour("ink", 0.85);
        context.fill();
        context.strokeStyle = mapColour("measure", 0.95);
        context.lineWidth = Math.max(1, dpr) * 1.4;
        context.stroke();
      }
    }
  }

  // A point placed but not yet joined: the mark, and a rubber line to the
  // pointer, so a half-made measurement looks half-made rather than lost.
  const pending = overlay.pending;
  if (pending) {
    const at = project(camera, view, { lon: pending[0], lat: pending[1] });
    if (!Number.isFinite(at.x) || !Number.isFinite(at.y)) {
      context.restore();
      return;
    }
    context.setLineDash([4 * dpr, 4 * dpr]);
    if (overlay.cursor) {
      context.beginPath();
      context.moveTo(at.x, at.y);
      context.lineTo(overlay.cursor.x, overlay.cursor.y);
      context.strokeStyle = mapColour("measure", 0.65);
      context.lineWidth = Math.max(1, dpr) * 1.4;
      context.stroke();
    }
    context.setLineDash([]);
    context.beginPath();
    context.arc(at.x, at.y, 4.5 * dpr, 0, Math.PI * 2);
    context.strokeStyle = mapColour("measure", 0.95);
    context.lineWidth = Math.max(1, dpr) * 1.4;
    context.stroke();
  }

  context.restore();
}

/**
 * Writes a label with a dark plate behind it.
 *
 * The map beneath is a colour ramp, so text on it is unreadable somewhere: a
 * plate is the only way a number stays legible over both calm water and a
 * gale.
 */
function writeLabel(
  context: CanvasRenderingContext2D,
  dpr: number,
  text: string,
  at: ScreenPoint,
  copies: readonly number[],
  muted = false,
): void {
  const padding = 3 * dpr;
  const width = context.measureText(text).width;
  const height = 13 * dpr;
  context.textAlign = "left";
  context.textBaseline = "middle";
  for (const offset of copies) {
    const x = at.x + offset + 7 * dpr;
    context.fillStyle = mapColour("ink", 0.78);
    context.fillRect(x - padding, at.y - height / 2 - padding, width + padding * 2, height + padding * 2);
    context.fillStyle = muted
      ? mapColour("measure", 0.8)
      : mapColour("measure", 1);
    context.fillText(text, x, at.y);
  }
}

/** What the option bar calls each kind. */
export const MEASURE_LABELS: Record<MeasurementKind, string> = {
  dividers: "Dividers",
  passage: "Great circle / rhumb",
  rings: "Range rings",
};

/** How many points a kind needs before it becomes a measurement. */
export function pointsNeeded(kind: MeasurementKind): number {
  switch (kind) {
    case "dividers":
      return 2;
    case "passage":
      return 2;
    case "rings":
      return 1;
  }
}
