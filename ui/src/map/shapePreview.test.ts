/**
 * The generic footprint: what each tool's gesture previews.
 *
 * Asserted against independently computed geography — a known ground distance,
 * a hand-checked containment — rather than against what the code returns, so
 * these fail if the preview stops matching the field rather than if it changes.
 */
import { describe, expect, it } from "vitest";

import type { Camera, Viewport } from "./camera";
import {
  buildFootprintPath,
  type Footprint,
  footprintHead,
  KM_PER_DEGREE,
  type PolygonSink,
} from "./footprint";
import { latticeUnder } from "./glyph";

const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 4 };
const view: Viewport = { width: 1440, height: 720 };

/** Records the calls a path would receive. */
class Recorder implements PolygonSink {
  readonly calls: string[] = [];
  readonly ellipses: Array<{ x: number; y: number; rx: number; ry: number }> = [];
  readonly rects: Array<{ x: number; y: number; w: number; h: number }> = [];
  readonly points: Array<{ x: number; y: number }> = [];

  moveTo(x: number, y: number) {
    this.calls.push("moveTo");
    this.points.push({ x, y });
  }
  lineTo(x: number, y: number) {
    this.calls.push("lineTo");
    this.points.push({ x, y });
  }
  closePath() {
    this.calls.push("closePath");
  }
  ellipse(x: number, y: number, rx: number, ry: number) {
    this.calls.push("ellipse");
    this.ellipses.push({ x, y, rx, ry });
  }
  rect(x: number, y: number, w: number, h: number) {
    this.calls.push("rect");
    this.rects.push({ x, y, w, h });
  }
}

function trace(footprint: Footprint): Recorder {
  const sink = new Recorder();
  buildFootprintPath(sink, camera, view, footprint);
  return sink;
}

describe("buildFootprintPath", () => {
  /**
   * The bug `footprint.ts` exists to prevent, now for the shapes that are not
   * strokes: `ellipse` continues the current subpath, so a ring drawn without a
   * `moveTo` before its inner circle joins the two by a straight line and fills
   * as a disc with a slot cut out of it.
   */
  it("opens a subpath before every circle of a ring", () => {
    const sink = trace({
      kind: "ring",
      centre: [0, 0],
      radiusKm: 1000,
      halfWidthKm: 200,
      space: "geodesic",
    });
    expect(sink.calls).toEqual(["moveTo", "ellipse", "moveTo", "ellipse"]);
  });

  /** Outer and inner radii are the centreline plus and minus the half-width. */
  it("draws a ring as two circles about its centreline", () => {
    const sink = trace({
      kind: "ring",
      centre: [0, 0],
      radiusKm: 1000,
      halfWidthKm: 200,
      space: "geodesic",
    });
    const [outer, inner] = sink.ellipses;
    const perKm = camera.pxPerDeg / KM_PER_DEGREE;
    expect(outer!.ry).toBeCloseTo(1200 * perKm, 6);
    expect(inner!.ry).toBeCloseTo(800 * perKm, 6);
  });

  /**
   * A ring wider than its own radius has no hole. Drawing the inner circle
   * anyway would give it a negative radius, which `ellipse` rejects.
   */
  it("omits the hole when the ring is thicker than its radius", () => {
    const sink = trace({
      kind: "ring",
      centre: [0, 0],
      radiusKm: 300,
      halfWidthKm: 400,
      space: "geodesic",
    });
    expect(sink.ellipses).toHaveLength(1);
  });

  /**
   * A geodesic shape covers `1/cos(lat)` as much longitude as it does latitude,
   * so it draws wider than it is tall away from the equator; a projected one is
   * a shape on the map and does not (spec.md 3.5).
   */
  it("stretches a geodesic rectangle east-west and leaves a projected one alone", () => {
    const at = (space: "geodesic" | "projected") =>
      trace({
        kind: "rect",
        centre: [0, 60],
        halfWidthKm: 500,
        halfHeightKm: 500,
        space,
      }).rects[0]!;

    const ground = at("geodesic");
    expect(ground.w / ground.h).toBeCloseTo(1 / Math.cos((60 * Math.PI) / 180), 3);

    const map = at("projected");
    expect(map.w / map.h).toBeCloseTo(1, 6);
  });

  it("closes a polygon rather than leaving it open", () => {
    const sink = trace({
      kind: "polygon",
      points: [
        [-2, -2],
        [2, -2],
        [2, 2],
      ],
    });
    expect(sink.calls).toEqual(["moveTo", "lineTo", "lineTo", "closePath"]);
  });

  it("draws nothing for a polygon with no vertices", () => {
    expect(trace({ kind: "polygon", points: [] }).calls).toEqual([]);
  });
});

describe("latticeUnder", () => {
  const step = 2;

  /**
   * The count is checked against the area the shape covers rather than against
   * a recorded number: a disc of radius r on a lattice of spacing s holds about
   * `pi r^2 / s^2` points, and anything that got the containment test wrong
   * misses that by a wide margin.
   */
  it("fills a disc with about as many glyphs as its area holds", () => {
    const radiusKm = 1000;
    const points = latticeUnder(
      { kind: "disc", centre: [0, 0], radiusKm, space: "geodesic" },
      step,
      10_000,
    );

    const radiusDeg = radiusKm / KM_PER_DEGREE;
    const expected = (Math.PI * radiusDeg * radiusDeg) / (step * step);
    expect(points.length).toBeGreaterThan(expected * 0.8);
    expect(points.length).toBeLessThan(expected * 1.2);

    // ...and every one of them really is inside.
    for (const [lon, lat] of points) {
      expect(Math.hypot(lon, lat)).toBeLessThanOrEqual(radiusDeg + 1e-9);
    }
  });

  /** The hole is the whole point of a ring. */
  it("leaves the middle of a ring empty", () => {
    const points = latticeUnder(
      {
        kind: "ring",
        centre: [0, 0],
        radiusKm: 1500,
        halfWidthKm: 200,
        space: "geodesic",
      },
      step,
      10_000,
    );
    expect(points.length).toBeGreaterThan(0);

    const inner = (1500 - 200) / KM_PER_DEGREE;
    for (const [lon, lat] of points) {
      expect(Math.hypot(lon, lat)).toBeGreaterThan(inner - step);
    }
  });

  /**
   * A concave polygon is where a containment test that used a bounding box, or
   * a winding rule with the notch wound the wrong way, would put glyphs in the
   * gap. The fixture is a U: the notch is empty and the arms are not.
   */
  it("keeps glyphs out of a concave polygon's notch", () => {
    const points = latticeUnder(
      {
        kind: "polygon",
        points: [
          [-6, -6],
          [6, -6],
          [6, 6],
          [2, 6],
          [2, -2],
          [-2, -2],
          [-2, 6],
          [-6, 6],
        ],
      },
      1,
      10_000,
    );
    expect(points.length).toBeGreaterThan(0);

    const inNotch = points.filter(
      ([lon, lat]) => lon > -2 && lon < 2 && lat > -2 && lat < 6,
    );
    expect(inNotch).toHaveLength(0);

    // The arms are covered, so the emptiness above is the notch and not the
    // whole polygon being missed.
    const inArm = points.filter(([lon, lat]) => lon > 3 && lon < 5 && lat > 0 && lat < 5);
    expect(inArm.length).toBeGreaterThan(0);
  });

  /** The cap is honoured, so a huge shape cannot stall a pointer move. */
  it("stops at the limit", () => {
    const points = latticeUnder(
      { kind: "disc", centre: [0, 0], radiusKm: 6000, space: "geodesic" },
      1,
      40,
    );
    expect(points).toHaveLength(40);
  });

  /**
   * A shape wider than the world must not walk the same columns twice: the
   * lattice wraps, so one pass round it reaches every point there is.
   */
  it("visits each lattice column once for a shape that spans the globe", () => {
    const points = latticeUnder(
      { kind: "disc", centre: [0, 0], radiusKm: 30_000, space: "geodesic" },
      10,
      100_000,
    );
    const keys = new Set(points.map(([lon, lat]) => `${lon},${lat}`));
    expect(keys.size).toBe(points.length);
  });
});

describe("footprintHead", () => {
  /** The fallback glyph must land on the shape, and a ring's centre is a hole. */
  it("puts a ring's fallback glyph on the ring rather than in its hole", () => {
    const footprint: Footprint = {
      kind: "ring",
      centre: [10, 20],
      radiusKm: 1000,
      halfWidthKm: 100,
      space: "geodesic",
    };
    const head = footprintHead(footprint);
    expect(head).not.toBeNull();
    const [lon, lat] = head!;
    // On the centreline: one radius north of the centre.
    expect(lon).toBeCloseTo(10, 6);
    expect(lat - 20).toBeCloseTo(1000 / KM_PER_DEGREE, 6);
  });

  it("takes the last point of a stroke, which is where the pointer is", () => {
    expect(
      footprintHead({
        kind: "swept",
        points: [
          [0, 0],
          [3, 4],
        ],
        radiusKm: 100,
        shape: "circle",
        space: "geodesic",
      }),
    ).toEqual([3, 4]);
  });

  it("has nothing to offer for an empty gesture", () => {
    expect(footprintHead({ kind: "polygon", points: [] })).toBeNull();
    expect(
      footprintHead({
        kind: "swept",
        points: [],
        radiusKm: 100,
        shape: "circle",
        space: "geodesic",
      }),
    ).toBeNull();
  });
});
