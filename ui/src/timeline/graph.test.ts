/**
 * The track value graph's arithmetic (spec.md 9.3).
 *
 * Checked against hand-computed numbers: a known knots conversion, a turn
 * whose arc is known, and pixel positions worked out from the geometry rather
 * than read back from the code.
 */
import { describe, expect, it } from "vitest";

import type { TrackSeries } from "../generated/TrackSeries";
import {
  extentOf,
  formatValue,
  normaliseDegrees,
  plotSeries,
  pointsOf,
  polyline,
  unwrapDegrees,
} from "./graph";

const series = (unit: string, values: number[], label = ""): TrackSeries => ({
  label,
  unit,
  values,
});

describe("plotSeries", () => {
  /** One knot is 1852 m per hour; 10 m/s is 36 km/h, or 19.438 kn. */
  it("graphs a speed in knots", () => {
    const plotted = plotSeries(series("speed", [0, 10]), "toward");
    expect(plotted.values[0]).toBe(0);
    expect(plotted.values[1]).toBeCloseTo((10 * 3600) / 1852, 6);
  });

  /**
   * Spec 3.3: a flow direction is stored as an azimuth-toward and shown in the
   * project's convention. A wind blowing toward 90° is a wind *from* 270°.
   */
  it("graphs a flow direction in the project's convention", () => {
    expect(plotSeries(series("direction", [90]), "from").values).toEqual([270]);
    expect(plotSeries(series("direction", [90]), "toward").values).toEqual([90]);
  });

  /** A geometric angle is not a flow direction and is graphed as stored. */
  it("leaves a geometric angle alone whatever the convention", () => {
    expect(plotSeries(series("degrees", [90]), "from").values).toEqual([90]);
  });

  it("copies a plain number through untouched", () => {
    expect(plotSeries(series("kilometres", [800, 400]), "from").values).toEqual([800, 400]);
  });
});

describe("unwrapDegrees", () => {
  /**
   * A shortest-arc turn from 350° to 10° is 20° clockwise, so the second
   * sample belongs at 370 — not 340° back down the graph.
   */
  it("follows the short way round rather than falling through the seam", () => {
    expect(unwrapDegrees([350, 10])).toEqual([350, 370]);
    expect(unwrapDegrees([10, 350])).toEqual([10, -10]);
  });

  it("accumulates across several turns", () => {
    expect(unwrapDegrees([300, 40, 140, 240, 340, 80])).toEqual([300, 400, 500, 600, 700, 800]);
  });

  it("leaves a series that never crosses the seam where it was", () => {
    expect(unwrapDegrees([10, 40, 100])).toEqual([10, 40, 100]);
  });

  it("does not disturb a latitude, which cannot wrap", () => {
    expect(unwrapDegrees([-80, 0, 80])).toEqual([-80, 0, 80]);
  });
});

describe("extentOf", () => {
  it("covers every series", () => {
    expect(
      extentOf([
        { label: "Lon", unit: "degrees", values: [-30, 10] },
        { label: "Lat", unit: "degrees", values: [4, 51] },
      ]),
    ).toEqual({ min: -30, max: 51 });
  });

  /** A constant track is a flat line through the middle, not a divide by zero. */
  it("pads a series that never changes", () => {
    expect(extentOf([{ label: "", unit: "none", values: [7, 7, 7] }])).toEqual({ min: 6, max: 8 });
  });
});

describe("pointsOf", () => {
  /**
   * A sample sits at the centre of its step's cell, so it lines up with the
   * ruler tick and the keyframe diamond above it. The maximum sits `pad` from
   * the top and the minimum `pad` from the bottom.
   */
  it("puts each sample at its cell centre, scaled to the extent", () => {
    const points = pointsOf([0, 5, 10], 10, 44, { min: 0, max: 10 }, 4);
    expect(points.map((p) => p.x)).toEqual([5, 15, 25]);
    expect(points.map((p) => p.y)).toEqual([40, 22, 4]);
  });

  it("writes them as an SVG points list", () => {
    expect(polyline([{ x: 5, y: 40 }, { x: 15, y: 4 }])).toBe("5.0,40.0 15.0,4.0");
  });
});

describe("formatValue", () => {
  it("carries the unit and keeps precision where it matters", () => {
    expect(formatValue("speed", 12.345)).toBe("12.3 kt");
    expect(formatValue("kilometres", 800)).toBe("800 km");
    expect(formatValue("percent", 0.25)).toBe("0.25%");
  });

  /** The plotted line may run past a turn to stay continuous; 370° is not a
   * direction anyone reads. */
  it("brings a graphed angle back inside one turn", () => {
    expect(formatValue("direction", 370)).toBe("10.0°");
    expect(normaliseDegrees(-10)).toBe(350);
  });
});
