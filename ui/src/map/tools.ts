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
import type { Camera } from "./camera";
import {
  cosLat,
  type Footprint,
  KM_PER_DEGREE,
  kmFromPixels,
  pixelsFromKm,
} from "./footprint";

/** The hand tool, which draws nothing and is not in the palette. */
export const HAND = "hand" as const;

/** What a pointer drag does: pan and select, or draw with one of the tools. */
export type ActiveTool = typeof HAND | Tool;

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

/** Whether a tool has any option measured in kilometres. */
export function hasSize(schema: ToolSchema): boolean {
  return schema.options.some((spec) => spec.unit === "kilometres");
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
  // (spec.md 3.5). Appended last so it overrides anything the bar was holding:
  // the unit selector is the control, and a stale value behind it is not.
  if (hasSize(schema)) {
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
      // footprint that changes as the drag moves north.
      const radiusKm = size("SizeKm", first[1]) / 2;
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
