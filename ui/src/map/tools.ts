/**
 * Tool state: the options a tool currently holds, and what they mean.
 *
 * The tools are described by Rust (`ve_app::palette`), which reports what
 * `ve_core::schema` already says. Nothing here decides which options a tool has
 * or which of them a mode makes inert — it resolves the rules the backend sent
 * against the values the bar is holding, which are the one thing the backend
 * does not have while a gesture is being set up.
 *
 * Pure, and free of React, so the rules can be tested against the schema rather
 * than by clicking the bar.
 */

import type { Gesture } from "../generated/Gesture";
import type { NewObject } from "../generated/NewObject";
import type { PropertyValue } from "../generated/PropertyValue";
import type { Tool } from "../generated/Tool";
import type { ToolOption } from "../generated/ToolOption";
import type { ToolOptionSpec } from "../generated/ToolOptionSpec";
import type { ToolSchema } from "../generated/ToolSchema";
import type { StampSpace } from "../generated/StampSpace";
import { displayDirection } from "../project/format";
import { type Camera, normalizeLon, projectionFor } from "./camera";
import {
  cosLat,
  type Footprint,
  KM_PER_DEGREE,
  kmFromPixels,
  pixelsFromKm,
} from "./footprint";

/** The hand tool, which draws nothing and is not in the palette. */
export const HAND = "hand" as const;

/**
 * The select tool: it draws a *region* of ground rather than an object
 * (spec.md 8.2, M14).
 *
 * Not in the backend's palette, and deliberately: the palette describes
 * vector-creation tools, and every entry in it maps to a `ToolKind` an object
 * can be made of. A region is a way of pointing, so it has no properties for
 * a schema to describe and no object for them to live on.
 */
export const SELECT = "select" as const;

/**
 * The measurement tool: it draws an *annotation* over the map (spec.md 10, M8).
 *
 * Frontend-only for the third time, and for the plainest reason yet: a
 * measurement is not an object at all. It contributes nothing to the field,
 * reaches no exported file, and has no properties for a schema to describe —
 * it is something drawn on the map to read a number off it.
 */
export const MEASURE = "measure" as const;

/**
 * The capture tool: it records a run of frames into the macro library
 * (spec.md 8.7, M16).
 *
 * Frontend-only like the region tools, and for the same reason: it makes no
 * object. What it makes is a file in the library, and the object that replays
 * one is the insert tool's.
 */
export const CAPTURE = "capture" as const;

/**
 * The insert tool: it puts a library macro down as an object (spec.md 8.7).
 *
 * The object it makes is a `ToolKind::Macro`, which is *not* in the backend's
 * palette — a macro has no schema bar of options to draw, it has a library to
 * choose from. So the tool is here and the object is there.
 */
export const INSERT = "insert" as const;

/** What a pointer drag does: pan and select, draw a region, measure, capture,
 * insert, or draw with one of the tools. */
export type ActiveTool =
  | typeof HAND
  | typeof SELECT
  | typeof MEASURE
  | typeof CAPTURE
  | typeof INSERT
  | Tool;

/** Whether a tool draws or edits objects rather than pointing at ground. */
export function drawsObjects(tool: ActiveTool): tool is Tool {
  return (
    tool !== HAND &&
    tool !== SELECT &&
    tool !== MEASURE &&
    tool !== CAPTURE &&
    tool !== INSERT
  );
}

/** The options a tool is holding, keyed by property id. */
export type ToolValues = Record<string, PropertyValue>;

/**
 * How a tool's sizes are being entered.
 *
 * One unit per tool rather than one per field, because `stamp_space` is one
 * property per object: a circle with a diameter in px and a ring width in km
 * would be asking for a shape that is on the map in one measurement and on the
 * ground in the other, and there is no such shape (spec.md 3.5).
 */
export type SizeUnit = "km" | "px";

/** Everything a tool is currently set to. */
export interface ToolState {
  /**
   * Option values. A `kilometres` option holds the number **as entered**, in
   * whatever {@link ToolState.unit} says; it becomes kilometres when the
   * gesture commits and never again (spec.md 3.5).
   */
  values: ToolValues;
  unit: SizeUnit;
}

/** The variant index a choice option holds, or 0. */
export function choiceOf(values: ToolValues, property: string): number {
  const value = values[property];
  return value?.kind === "choice" ? value.index : 0;
}

/** The number a scalar option holds, or 0. */
export function numberOf(values: ToolValues, property: string): number {
  const value = values[property];
  return value?.kind === "number" ? value.value : 0;
}

/** The degrees an angle option holds, or 0. */
export function angleOf(values: ToolValues, property: string): number {
  const value = values[property];
  return value?.kind === "angle" ? value.degrees : 0;
}

/** The position an option holds, or the origin. */
export function positionOf(values: ToolValues, property: string): [number, number] {
  const value = values[property];
  return value?.kind === "position" ? [value.lon, value.lat] : [0, 0];
}

/**
 * Whether an option is read at all, given the modes the tool is in.
 *
 * The mirror of `ve_core::schema::is_live`, and deliberately the same shape:
 * every rule must hold, so two rules on one option — as the shape fill has —
 * and together rather than compete (spec.md 6.1).
 */
export function isLive(spec: ToolOptionSpec, values: ToolValues): boolean {
  return spec.depends_on.every((rule) => rule.live_for.includes(choiceOf(values, rule.on)));
}

/** The options the bar should show, in order. */
export function liveOptions(schema: ToolSchema, values: ToolValues): ToolOptionSpec[] {
  return schema.options.filter((spec) => isLive(spec, values));
}

/** The state a tool starts in: every option at its schema default. */
export function defaultState(schema: ToolSchema): ToolState {
  const values: ToolValues = {};
  for (const spec of schema.options) values[spec.property] = spec.default;
  return { values, unit: "km" };
}

/**
 * Whether the tool's unit control is live, given the modes it is in.
 *
 * The unit *is* the stamp-space control (spec.md 3.5), so it carries that
 * property's dependency rules: the shape fill's freehand polygon has no size,
 * and so is offered no unit to measure one in.
 */
export function offersUnit(schema: ToolSchema, values: ToolValues): boolean {
  const sizing = schema.sizing;
  if (!sizing) return false;
  return sizing.depends_on.every((rule) => rule.live_for.includes(choiceOf(values, rule.on)));
}

/**
 * Whether the tool's eyedropper is live, given the modes it is in.
 *
 * Same rule as every other dependency, against the same values: the eyedropper
 * writes one speed and one bearing, so it is offered exactly where the tool
 * paints one of each — not in a gradient mode, which has two, and not where a
 * curve's bearing is an offset from its own path rather than a direction
 * (spec.md 6.1).
 */
export function offersEyedropper(schema: ToolSchema, values: ToolValues): boolean {
  const eyedropper = schema.eyedropper;
  if (!eyedropper) return false;
  return eyedropper.depends_on.every((rule) => rule.live_for.includes(choiceOf(values, rule.on)));
}

/**
 * The tool values an eyedropper's sample writes.
 *
 * Stored units, not shown ones: a speed in m/s and an azimuth-toward, exactly
 * as the backend reported them and as the object will hold them. The option bar
 * converts for display like it does for every other value, so the sample takes
 * the same path a typed number does (spec.md 3.3).
 */
export function sampled(
  state: ToolState,
  schema: ToolSchema,
  sample: { speed_mps: number; azimuth_toward_deg: number },
): ToolState {
  const eyedropper = schema.eyedropper;
  if (!eyedropper) return state;
  return {
    ...state,
    values: {
      ...state.values,
      [eyedropper.speed]: { kind: "number", value: sample.speed_mps },
      [eyedropper.direction]: { kind: "angle", degrees: sample.azimuth_toward_deg },
    },
  };
}

/**
 * The stamp space a unit selects.
 *
 * px asks for a shape on the map and km for one on the ground; the unit is not
 * just a conversion (spec.md 3.5). This is the whole of the rule, in one place,
 * for every tool that has a size.
 */
export function spaceFor(unit: SizeUnit): StampSpace {
  return unit === "px" ? "projected" : "geodesic";
}

/**
 * A size option in kilometres, resolved against where the gesture is.
 *
 * A number entered in px is a number of pixels *now*, at *this* latitude, and
 * becomes a ground distance at the moment the object is created — never
 * revisited, so zooming afterwards cannot resize what was drawn.
 */
export function sizeKm(
  state: ToolState,
  property: string,
  camera: Camera,
  lat: number,
): number {
  const amount = numberOf(state.values, property);
  if (state.unit === "km") return amount;
  return kmFromPixels(camera, lat, amount, spaceFor(state.unit));
}

/**
 * Carries a size across a change of unit.
 *
 * Switching the unit must not resize the tool, so the number is converted
 * rather than reinterpreted. The unit also changes the *space*, so the two
 * sizes describe different stamps; what is carried across is the north-south
 * extent, which is the axis both spaces share and therefore the one that keeps
 * the footprint the same height on screen through the switch.
 */
export function convertSizes(
  state: ToolState,
  schema: ToolSchema,
  next: SizeUnit,
  camera: Camera,
  lat: number,
): ToolState {
  if (next === state.unit) return state;
  const values: ToolValues = { ...state.values };
  for (const spec of schema.options) {
    if (spec.unit !== "kilometres") continue;
    const amount = numberOf(state.values, spec.property);
    // Both conversions are made in the *projected* space, in both directions,
    // because that is the space the pixel number lives in: px selects
    // `projected`, so a size in pixels is always a size on the map. Converting
    // the km side in its own space instead scales by `cos(lat)` and the tool
    // changes size on the way through — a fifth of it at 40°.
    const converted =
      next === "px"
        ? pixelsFromKm(camera, lat, amount, "projected")
        : kmFromPixels(camera, lat, amount, "projected");
    values[spec.property] = { kind: "number", value: Math.max(1, Math.round(converted)) };
  }
  return { values, unit: next };
}

/**
 * A stored angle as the bar shows it, and back again.
 *
 * A flow direction is stored as an azimuth-toward and shown in the project's
 * convention; a geometric bearing is not converted (spec.md 3.3). The same
 * distinction the inspector makes, made here from the same unit — and the
 * conversion is its own inverse, so one function serves both ways.
 */
export function shownAngle(unit: string, convention: string, degrees: number): number {
  return unit === "direction" ? displayDirection(convention, degrees) : degrees;
}

/** Which gesture drives the tool, given the modes it is in. */
export function gestureKind(schema: ToolSchema, values: ToolValues): Gesture["kind"] {
  const selector = schema.gesture;
  if (selector.kind === "always") return selector.gesture as Gesture["kind"];
  const index = choiceOf(values, selector.on);
  return (selector.gestures[index] ?? selector.gestures[0] ?? "stroke") as Gesture["kind"];
}

/**
 * The options to freeze onto the object.
 *
 * Sizes are sent in kilometres, resolved against the gesture's own latitude;
 * everything else goes as it is held. Inert options are sent too: hidden is not
 * deleted, and a value the mode does not read still has to be on the object so
 * that switching the mode later brings it back as it was (spec.md 6.1).
 */
export function frozenOptions(
  state: ToolState,
  schema: ToolSchema,
  camera: Camera,
  lat: number,
): ToolOption[] {
  const options: ToolOption[] = [];
  for (const spec of schema.options) {
    if (spec.unit === "kilometres") {
      options.push({
        property: spec.property,
        value: {
          kind: "number",
          value: Math.max(1, sizeKm(state, spec.property, camera, lat)),
        },
      });
      continue;
    }
    const value = state.values[spec.property];
    if (value !== undefined) options.push({ property: spec.property, value });
  }

  // The unit chose the space, and the space is a property of the object
  // (spec.md 3.5). Sent from the unit and never from the bar's values, because
  // the unit is the only control there is: the tool offers no stamp-space
  // option of its own, so there is nothing else it could come from.
  //
  // Sent even where the mode makes it inert — a polygon's, say. Hidden is not
  // deleted, and the object still carries the property at a known value.
  if (schema.sizing) {
    const space = spaceFor(state.unit);
    options.push({
      property: "StampSpace",
      value: { kind: "choice", index: space === "projected" ? 1 : 0 },
    });
  }
  return options;
}

/** The payload for one finished gesture. */
export function newObject(
  tool: Tool,
  gesture: Gesture,
  state: ToolState,
  schema: ToolSchema,
  camera: Camera,
  lat: number,
  layer: number | null,
): NewObject {
  return {
    tool,
    gesture,
    options: frozenOptions(state, schema, camera, lat),
    ...(layer !== null ? { layer } : {}),
  };
}

/**
 * The footprint a gesture in progress will paint.
 *
 * The single place a tool's geometry becomes something the overlay can draw.
 * Every preview, hover indicator and held preview goes through it, so a tool
 * that gets this right inherits all three and one that gets it wrong is wrong
 * in all three at once — which is the failure worth having, since it shows up
 * immediately rather than in whichever of the three nobody tried.
 *
 * `null` when there is nothing to draw yet: a polygon of one vertex, a drag
 * that has not moved.
 */
export function footprintOf(
  tool: Tool,
  state: ToolState,
  gesture: Gesture,
  camera: Camera,
): Footprint | null {
  const space = spaceFor(state.unit);
  const size = (property: string, lat: number) => sizeKm(state, property, camera, lat);

  switch (gesture.kind) {
    case "stroke": {
      const first = gesture.points[0];
      if (first === undefined) return null;
      // Resolved at the stroke's first point, which is the latitude the commit
      // will use — so what is previewed is what gets painted, rather than a
      // footprint that changes as the drag moves north. A curve's hover is a
      // one-point stroke too, and its stamp's size is its width (M24).
      const radiusKm = size(tool === "curve" ? "WidthKm" : "SizeKm", first[1]) / 2;
      return {
        kind: "swept",
        points: gesture.points,
        radiusKm,
        shape: choiceOf(state.values, "BrushShape") === 1 ? "square" : "circle",
        space,
      };
    }

    case "point": {
      const [lon, lat] = gesture.at;
      const radiusKm = size("DiameterKm", lat) / 2;
      // Fill mode 1 is the perimeter ring; the other two are filled.
      if (tool === "circle" && choiceOf(state.values, "FillMode") === 1) {
        return {
          kind: "ring",
          centre: [lon, lat],
          radiusKm,
          halfWidthKm: size("RingWidthKm", lat) / 2,
          space,
        };
      }
      return { kind: "disc", centre: [lon, lat], radiusKm, space };
    }

    case "extent": {
      const [lon, lat] = gesture.centre;
      const [rimLon, rimLat] = gesture.rim;
      // The drag's reach, measured on the map in the same terms the backend
      // measures it in the object's frame.
      const { halfWidthKm, halfHeightKm } = extentOf(
        [lon, lat],
        [rimLon, rimLat],
        space,
      );
      if (halfWidthKm <= 0 && halfHeightKm <= 0) return null;

      // Only the shape fill has a `shape_source`. For every other tool an
      // extent is a disc — a selected circle applied with an operator
      // (spec.md 8.2) — and the preview has to say the same as the backend.
      if (tool !== "shape_fill") {
        return {
          kind: "disc",
          centre: [lon, lat],
          radiusKm: Math.hypot(halfWidthKm, halfHeightKm),
          space,
        };
      }

      // Shape source 1 square, 2 rectangle, 3 circle.
      switch (choiceOf(state.values, "ShapeSource")) {
        case 1: {
          const half = Math.max(halfWidthKm, halfHeightKm);
          return {
            kind: "rect",
            centre: [lon, lat],
            halfWidthKm: half,
            halfHeightKm: half,
            space,
          };
        }
        case 3:
          return {
            kind: "disc",
            centre: [lon, lat],
            radiusKm: Math.hypot(halfWidthKm, halfHeightKm),
            space,
          };
        default:
          return { kind: "rect", centre: [lon, lat], halfWidthKm, halfHeightKm, space };
      }
    }

    case "ring":
      // A polygon needs three vertices to have an inside; below that the
      // gesture draws its edges and nothing is filled.
      return gesture.points.length >= 3
        ? { kind: "polygon", points: gesture.points }
        : null;

    case "path": {
      const first = gesture.nodes[0];
      if (first === undefined || gesture.nodes.length < 2) return null;
      return {
        kind: "swept",
        points: flattenPath(gesture.nodes),
        radiusKm: size("WidthKm", first.at[1]) / 2,
        shape: "circle",
        space,
      };
    }
  }
}

/**
 * Half-extents of a centre-out drag, in kilometres.
 *
 * The east-west number is a *ground* distance for a geodesic shape and a
 * north-equivalent one for a projected shape, which is the same distinction the
 * object's frame makes (spec.md 7.2) — so the preview and the committed object
 * measure the drag the same way.
 */
export function extentOf(
  centre: readonly [number, number],
  rim: readonly [number, number],
  space: StampSpace,
): { halfWidthKm: number; halfHeightKm: number } {
  const dLat = rim[1] - centre[1];
  let dLon = rim[0] - centre[0];
  // The shorter way round, so a drag across the dateline stays a drag.
  dLon = ((dLon + 540) % 360) - 180;
  const scale = space === "projected" ? 1 : cosLat(centre[1]);
  return {
    halfWidthKm: Math.abs(dLon) * KM_PER_DEGREE * scale,
    halfHeightKm: Math.abs(dLat) * KM_PER_DEGREE,
  };
}

/**
 * A path's nodes as a polyline, expanding Bézier segments.
 *
 * The preview's own flattening, at a resolution chosen for the screen rather
 * than for the field: `ve_render::scene::flatten_path` subdivides to 250 m,
 * which is far finer than any preview can show and far more work than a pointer
 * move can afford. The two need to agree perceptually, not numerically
 * (invariant 3).
 */
export function flattenPath(
  nodes: ReadonlyArray<{
    at: [number, number];
    in_handle?: [number, number] | null;
    out_handle?: [number, number] | null;
  }>,
): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  const first = nodes[0];
  if (first === undefined) return out;
  out.push([first.at[0], first.at[1]]);

  /** Enough that a segment reads as a curve at any zoom a preview is drawn at. */
  const STEPS = 24;

  for (let i = 1; i < nodes.length; i++) {
    const a = nodes[i - 1]!;
    const b = nodes[i]!;
    const from = a.at;
    const to = b.at;
    const c1 = a.out_handle ?? null;
    const c2 = b.in_handle ?? null;
    if (c1 === null && c2 === null) {
      out.push([to[0], to[1]]);
      continue;
    }
    const p1 = c1 ?? from;
    const p2 = c2 ?? to;
    for (let step = 1; step <= STEPS; step++) {
      const t = step / STEPS;
      const u = 1 - t;
      const w = [u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t];
      out.push([
        w[0]! * from[0] + w[1]! * p1[0] + w[2]! * p2[0] + w[3]! * to[0],
        w[0]! * from[1] + w[1]! * p1[1] + w[2]! * p2[1] + w[3]! * to[1],
      ]);
    }
  }
  return out;
}

/**
 * How a gesture's preview looks: the speed to colour it and the direction to
 * draw over it (spec.md 6.1).
 *
 * What the user is aiming is a wind, so what the tool shows while aiming is
 * that wind, in the terms the map already displays it in. This is *overlay*
 * drawing, not evaluation: the values come from the tool's own options rather
 * than from a tile, nothing is composited, and spec 7.9's fidelity tolerances
 * do not apply. A gradient is previewed at its mean and a path's flow at its
 * chord — approximations a preview is allowed and an export is not.
 */
export interface PreviewField {
  /** Speed in knots, for the ramp colour and the glyphs. */
  knots: number;
  /** The azimuth-toward a stamp at this position carries. */
  azimuthAt: (lon: number, lat: number) => number;
}

/** The speed in metres per second the preview should paint. */
function previewSpeedMps(tool: Tool, values: ToolValues): number {
  // The mask writes calm, and calm is what its preview should show — the
  // floor under the preview's opacity is what keeps it visible (spec.md 6.1).
  if (tool === "mask") return 0;

  // A gradient has no single speed. Its mean is the honest one number, and the
  // preview is explicitly a proxy rather than the composited answer.
  if (tool === "circle" && choiceOf(values, "FillMode") === 2) {
    return (numberOf(values, "SpeedMin") + numberOf(values, "SpeedMax")) / 2;
  }
  if (tool === "shape_fill" && choiceOf(values, "VectorMode") === 1) {
    return (numberOf(values, "SpeedStart") + numberOf(values, "SpeedEnd")) / 2;
  }
  return numberOf(values, "Speed");
}

/**
 * The bearing from one position to another, as a true initial azimuth.
 *
 * The same great-circle bearing the evaluator takes (`aeqd.rs`), so an aimed
 * preview points where the committed field will — a local frame angle would
 * drift by tens of degrees a few thousand kilometres out.
 */
function bearingTo(
  from: readonly [number, number],
  to: readonly [number, number],
): number {
  const toRad = Math.PI / 180;
  const phi1 = from[1] * toRad;
  const phi2 = to[1] * toRad;
  const dLambda = normalizeLon(to[0] - from[0]) * toRad;
  const y = Math.sin(dLambda) * Math.cos(phi2);
  const x =
    Math.cos(phi1) * Math.sin(phi2) - Math.sin(phi1) * Math.cos(phi2) * Math.cos(dLambda);
  return (Math.atan2(y, x) / toRad + 360) % 360;
}

/**
 * How the preview's glyphs should point, for any tool.
 *
 * Each branch mirrors the direction mode the evaluator will use, so the arrows
 * on screen cannot disagree with what gets painted — the one bug this whole
 * preview path exists to make impossible.
 */
export function previewField(
  tool: Tool,
  state: ToolState,
  footprint: Footprint | null,
): PreviewField {
  const values = state.values;
  const knots = previewSpeedMps(tool, values) * KNOTS_PER_MPS;
  const constant = () => angleOf(values, "Direction");

  // A curve aims along its own path; the offset is added to the local tangent.
  if (tool === "curve" && choiceOf(values, "CurveDirectionMode") === 1) {
    const offset = angleOf(values, "Direction");
    const points = footprint?.kind === "swept" ? footprint.points : [];
    return {
      knots,
      azimuthAt: (lon, lat) => (tangentAt(points, [lon, lat]) + offset + 360) % 360,
    };
  }

  // A circle's flow turns about its centre, a quarter turn off the outward
  // bearing, which side depending on the sense.
  if (tool === "circle") {
    const quarter = choiceOf(values, "RotationSense") === 0 ? 90 : -90;
    const centre =
      footprint && "centre" in footprint ? footprint.centre : ([0, 0] as const);
    return {
      knots,
      azimuthAt: (lon, lat) => (bearingTo(centre, [lon, lat]) + quarter + 360) % 360,
    };
  }

  // The shared aim modes, for every tool that has a target. Mode 1 points at
  // it and mode 2 directly away — the reciprocal at the cell, which is the
  // outward tangent to the same great circle rather than the bearing measured
  // at the target (spec.md 6.2).
  const aims = choiceOf(values, "DirectionMode");
  const gradient = tool === "shape_fill" && choiceOf(values, "VectorMode") === 1;
  if (!gradient && (aims === 1 || aims === 2)) {
    const target = positionOf(values, "Target");
    const turn = aims === 2 ? 180 : 0;
    return {
      knots,
      azimuthAt: (lon, lat) => (bearingTo([lon, lat], target) + turn + 360) % 360,
    };
  }

  // A gradient ramps between two bearings; its midpoint is the one bearing a
  // preview can show without evaluating the ramp per glyph.
  if (gradient) {
    const start = angleOf(values, "DirectionStart");
    const end = angleOf(values, "DirectionEnd");
    // The shortest arc, as everywhere else angles are interpolated.
    const delta = ((end - start + 540) % 360) - 180;
    const middle = (start + delta / 2 + 360) % 360;
    return { knots, azimuthAt: () => middle };
  }

  return { knots, azimuthAt: () => constant() };
}

/** Knots per metre per second. Mirrors `ve_core::units`. */
const KNOTS_PER_MPS = 1.943_844_49;

/** The bearing of the nearest segment of a polyline, in degrees. */
function tangentAt(
  points: ReadonlyArray<readonly [number, number]>,
  at: readonly [number, number],
): number {
  if (points.length < 2) return 0;
  let best = Infinity;
  let bearing = 0;
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1]!;
    const b = points[i]!;
    // Distance in degrees is enough to choose a segment; the bearing itself is
    // then taken on the globe, so it is a true azimuth (`aeqd.rs`).
    const ax = normalizeLon(at[0] - a[0]);
    const ay = at[1] - a[1];
    const bx = normalizeLon(b[0] - a[0]);
    const by = b[1] - a[1];
    const denom = bx * bx + by * by;
    const t = denom <= 1e-12 ? 0 : Math.max(0, Math.min(1, (ax * bx + ay * by) / denom));
    const distance = Math.hypot(ax - bx * t, ay - by * t);
    if (distance < best) {
      best = distance;
      bearing = bearingTo(a, b);
    }
  }
  return bearing;
}

/**
 * The camera a clone stamp's source is read through.
 *
 * The main camera shifted so that what is at the source appears where the brush
 * is. `null` when the gesture has nothing placed yet.
 *
 * The longitude shift is a plain translation, because longitude is linear in
 * `x` in every projection the map offers. The latitude shift is not: a constant
 * offset in degrees is a constant offset in pixels only under equirectangular
 * (spec.md 5.1). So the shift is made in the projection's own vertical
 * coordinate and anchored at the brush — exact where the user is drawing, and
 * increasingly approximate away from it, which is the same bargain the `Fixed`
 * mode below already makes.
 *
 * Which offset depends on the mode (spec.md 6.2). `Aligned` measures it from
 * the object's anchor — where the gesture began — and is exact everywhere.
 * `Fixed` measures it per cell from the nearest point of the stroke, which no
 * single translation can express; the preview uses the pointer's own position,
 * which is exact where the user is looking and approximate behind it. A preview
 * is allowed to approximate, and this one says where it does.
 */
export function cloneSourceCamera(
  state: ToolState,
  gesture: Gesture,
  camera: Camera,
): Camera | null {
  if (gesture.kind !== "stroke") return null;
  const points = gesture.points;
  const anchor = points[0];
  const pointer = points[points.length - 1];
  if (!anchor || !pointer) return null;

  // Offset mode 1 is the fixed source.
  const from = choiceOf(state.values, "OffsetMode") === 1 ? pointer : anchor;
  const [sourceLon, sourceLat] = positionOf(state.values, "SourcePoint");

  const projection = projectionFor(camera);
  const shiftY = projection.yOf(from[1]) - projection.yOf(sourceLat);
  return {
    ...camera,
    centerLon: camera.centerLon - normalizeLon(from[0] - sourceLon),
    centerLat: projection.latOf(projection.yOf(camera.centerLat) - shiftY),
  };
}
