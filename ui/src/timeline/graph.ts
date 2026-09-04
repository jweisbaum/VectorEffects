/**
 * The track value graph's arithmetic (spec.md 9.3).
 *
 * The backend samples a property at every step; this turns those samples into
 * the numbers a reader sees — display units, a continuous line across the
 * 360° seam, and pixel coordinates. Pure, so the scaling can be checked against
 * hand-computed positions rather than by looking at the screen.
 *
 * It converts and it does not interpolate: what happens *between* two keys is
 * the model's answer, arriving already sampled (`api.trackSamples`).
 */

import type { TrackSeries } from "../generated/TrackSeries";
import { displayDirection, knotsFromMps } from "../project/format";

/** A series ready to plot: display units, and continuous across the seam. */
export interface PlotSeries {
  /** Empty for a single-component property; otherwise which component. */
  label: string;
  /** The display unit its values are now in. */
  unit: string;
  values: number[];
}

/** The suffix a graphed value carries. Mirrors the inspector's. */
export function unitSuffix(unit: string): string {
  switch (unit) {
    case "speed":
      return " kn";
    case "kilometres":
      return " km";
    case "degrees":
    case "direction":
      return "°";
    case "percent":
      return "%";
    default:
      return "";
  }
}

/** Whether a unit is measured in degrees, and so wraps. */
function isAngular(unit: string): boolean {
  return unit === "degrees" || unit === "direction";
}

/**
 * Unwraps a degree series so consecutive samples never jump a seam.
 *
 * A direction interpolates along the shortest arc, so a turn from 350° to 10°
 * passes through 0 — and a graph plotted from the wrapped numbers draws that
 * 20° turn as a full-height fall. Each sample is moved by whole turns to sit
 * within half a turn of the one before, which draws the arc the model actually
 * took. The values leave `[0, 360)`; `normaliseDegrees` puts one back for a
 * readout.
 */
export function unwrapDegrees(values: readonly number[]): number[] {
  const out: number[] = [];
  let previous: number | null = null;
  for (const value of values) {
    if (previous === null) {
      out.push(value);
      previous = value;
      continue;
    }
    const turns = Math.round((value - previous) / 360);
    const next = value - turns * 360;
    out.push(next);
    previous = next;
  }
  return out;
}

/** A degree value back in `[0, 360)`, for a readout. */
export function normaliseDegrees(value: number): number {
  return ((value % 360) + 360) % 360;
}

/**
 * One sampled series as it is graphed.
 *
 * Speed is stored in m/s and always shown in knots; a flow direction is stored
 * as an azimuth-toward and shown in the project's convention. Both conversions
 * are the same ones the inspector and the map readout make — this is the IPC
 * boundary for a graph.
 */
export function plotSeries(series: TrackSeries, convention: string): PlotSeries {
  if (series.unit === "speed") {
    return { label: series.label, unit: series.unit, values: series.values.map(knotsFromMps) };
  }
  if (series.unit === "direction") {
    return {
      label: series.label,
      unit: series.unit,
      values: unwrapDegrees(series.values.map((v) => displayDirection(convention, v))),
    };
  }
  if (isAngular(series.unit)) {
    return { label: series.label, unit: series.unit, values: unwrapDegrees(series.values) };
  }
  return { label: series.label, unit: series.unit, values: [...series.values] };
}

/** The value range a graph is drawn against. */
export interface Extent {
  min: number;
  max: number;
}

/**
 * The extent covering every series, padded so a constant track is a flat line
 * through the middle rather than a division by zero.
 */
export function extentOf(series: ReadonlyArray<PlotSeries>): Extent {
  let min = Infinity;
  let max = -Infinity;
  for (const one of series) {
    for (const value of one.values) {
      if (!Number.isFinite(value)) continue;
      if (value < min) min = value;
      if (value > max) max = value;
    }
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) return { min: 0, max: 1 };
  if (max - min < 1e-9) return { min: min - 1, max: max + 1 };
  return { min, max };
}

/** A point on the graph, in the SVG's own pixels. */
export interface Point {
  x: number;
  y: number;
}

/**
 * Where each sample sits, in the SVG's pixels.
 *
 * `x` is the centre of the step's cell, so a sample lines up with its ruler
 * tick and its keyframe diamond. `y` runs from `pad` at the extent's maximum
 * to `height - pad` at its minimum — up the screen is up the scale.
 */
export function pointsOf(
  values: readonly number[],
  pxPerStep: number,
  height: number,
  extent: Extent,
  pad = 4,
): Point[] {
  const span = extent.max - extent.min;
  const usable = Math.max(1, height - pad * 2);
  return values.map((value, step) => ({
    x: (step + 0.5) * pxPerStep,
    y: pad + (1 - (value - extent.min) / span) * usable,
  }));
}

/** Those points as an SVG `points` attribute. */
export function polyline(points: readonly Point[]): string {
  return points.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ");
}

/**
 * A graphed value, with its unit.
 *
 * Angles are normalised back into `[0, 360)`: the plotted line may run past a
 * turn to stay continuous, but "370°" is not a direction anyone reads.
 */
export function formatValue(unit: string, value: number): string {
  const shown = isAngular(unit) ? normaliseDegrees(value) : value;
  const magnitude = Math.abs(shown);
  const digits = magnitude >= 100 ? 0 : magnitude >= 10 ? 1 : 2;
  return `${shown.toFixed(digits)}${unitSuffix(unit)}`;
}
