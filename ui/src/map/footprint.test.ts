import { PROJECTIONS } from "./projection";
import { describe, expect, it } from "vitest";

import {
  addFootprint,
  buildStrokePath,
  footprintRadii,
  KM_PER_DEGREE,
  kmFromPixels,
  pixelsFromKm,
  type PathSink,
  footprintOfOutline,
} from "./footprint";
import { cameraForProjection } from "./camera";
import type { Camera, Viewport } from "./camera";

const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 4 };
const view: Viewport = { width: 800, height: 600 };

/** Records the calls a real Path2D would receive. */
function recorder(): PathSink & { calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    moveTo: () => calls.push("moveTo"),
    lineTo: () => calls.push("lineTo"),
    closePath: () => calls.push("closePath"),
    ellipse: () => calls.push("ellipse"),
    rect: () => calls.push("rect"),
  };
}

describe("footprintRadii", () => {
  /**
   * The point of a projected stamp: the same number of pixels in both axes,
   * wherever it sits. A geodesic one is an ellipse everywhere but the equator.
   */
  it("is circular at every latitude in map space", () => {
    for (const lat of [0, 30, 60, 85, -70]) {
      const { rx, ry } = footprintRadii(camera, lat, 500, "projected");
      expect(rx).toBeCloseTo(ry, 9);
    }
  });

  /** Both spaces agree on the north-south extent; only the width differs. */
  it("leaves the north-south extent alone", () => {
    const ground = footprintRadii(camera, 60, 500);
    const map = footprintRadii(camera, 60, 500, "projected");
    expect(map.ry).toBeCloseTo(ground.ry, 9);
    expect(map.rx).toBeCloseTo(ground.rx / 2, 3);
  });

  it("is circular at the equator", () => {
    const { rx, ry } = footprintRadii(camera, 0, KM_PER_DEGREE);
    expect(ry).toBeCloseTo(4, 6);
    expect(rx).toBeCloseTo(ry, 6);
  });

  /** A ground circle spans more longitude the further from the equator. */
  it("widens with latitude", () => {
    const equator = footprintRadii(camera, 0, 500);
    const high = footprintRadii(camera, 60, 500);
    expect(high.ry).toBeCloseTo(equator.ry, 9);
    expect(high.rx).toBeCloseTo(equator.rx * 2, 4);
  });

  it("stays finite at the pole", () => {
    const { rx, ry } = footprintRadii(camera, 90, 500);
    expect(Number.isFinite(rx)).toBe(true);
    expect(Number.isFinite(ry)).toBe(true);
    expect(rx).toBeLessThan(ry * 60);
  });
});

describe("path construction", () => {
  /**
   * The regression this file exists for. `ellipse` continues the current
   * subpath, so a missing `moveTo` joins consecutive footprints with a straight
   * line and the whole stroke fills as one enormous polygon.
   */
  it("starts a new subpath before every footprint", () => {
    const sink = recorder();
    buildStrokePath(sink, camera, view, [[0, 0], [5, 5], [10, 10]], 300);

    // The count depends on interpolation; the alternation does not, and it is
    // the alternation that matters.
    expect(sink.calls.length).toBeGreaterThanOrEqual(6);
    expect(sink.calls.length % 2).toBe(0);
    for (let i = 0; i < sink.calls.length; i += 2) {
      expect(sink.calls[i]).toBe("moveTo");
      expect(sink.calls[i + 1]).toBe("ellipse");
    }
  });

  it("adds one subpath per point", () => {
    const sink = recorder();
    buildStrokePath(sink, camera, view, [[0, 0]], 300);
    expect(sink.calls.filter((c) => c === "ellipse")).toHaveLength(1);
  });

  it("draws nothing for an empty stroke", () => {
    const sink = recorder();
    buildStrokePath(sink, camera, view, [], 300);
    expect(sink.calls).toEqual([]);
  });

  it("places the subpath start on the ellipse it opens", () => {
    const positions: Array<{ x: number; y: number }> = [];
    const sink: PathSink = {
      moveTo: (x, y) => positions.push({ x, y }),
      lineTo: () => {},
      closePath: () => {},
      ellipse: (x, y, rx) => positions.push({ x: x + rx, y }),
      rect: () => {},
    };
    addFootprint(sink, camera, view, 0, 0, 300);

    // The moveTo must land exactly where the ellipse begins, or the fill picks
    // up a stray edge.
    expect(positions[0]!.x).toBeCloseTo(positions[1]!.x, 9);
    expect(positions[0]!.y).toBeCloseTo(positions[1]!.y, 9);
  });
});

describe("stroke interpolation", () => {
  /**
   * The regression: stamping only the recorded points leaves visible gaps
   * whenever a drag outruns the pointer sample rate. The stored stroke is a
   * continuously swept capsule, so the *field* is solid either way — but the
   * preview looked broken.
   */
  it("fills a long segment with overlapping footprints", () => {
    const sink = recorder();
    // 40 degrees apart at 4 px/degree is 160 px, against a footprint radius of
    // about 18 px for a 500 km brush.
    buildStrokePath(sink, camera, view, [[0, 0], [40, 0]], 500);

    const stamps = sink.calls.filter((c) => c === "ellipse").length;
    expect(stamps).toBeGreaterThan(10);
  });

  it("leaves a short segment alone", () => {
    const sink = recorder();
    buildStrokePath(sink, camera, view, [[0, 0], [0.2, 0]], 500);
    expect(sink.calls.filter((c) => c === "ellipse").length).toBeLessThanOrEqual(3);
  });

  it("still opens a subpath for every interpolated footprint", () => {
    const sink = recorder();
    buildStrokePath(sink, camera, view, [[0, 0], [30, 10]], 400);

    // Strictly alternating: a missing moveTo would join footprints into one
    // enormous polygon.
    for (let i = 0; i < sink.calls.length; i += 2) {
      expect(sink.calls[i]).toBe("moveTo");
      expect(sink.calls[i + 1]).toBe("ellipse");
    }
  });

  /** Interpolating the long way round would sweep a band across the world. */
  it("crosses the antimeridian by the shorter way", () => {
    const longitudes: number[] = [];
    const sink: PathSink = {
      moveTo: () => {},
      lineTo: () => {},
      closePath: () => {},
      ellipse: () => {},
      rect: () => {},
    };
    // Capture the geographic input by spying on the projection indirectly:
    // a short crossing must produce few stamps, a wrong one very many.
    buildStrokePath(
      {
        moveTo: () => {},
        lineTo: () => {},
        closePath: () => {},
        ellipse: () => longitudes.push(0),
        rect: () => {},
      },
      camera,
      view,
      [[179, 0], [-179, 0]],
      500,
    );
    void sink;
    // Two degrees apart: a handful of stamps. Going the wrong way would be 358
    // degrees and hit the cap.
    expect(longitudes.length).toBeLessThan(20);
  });

  it("caps the number of footprints", () => {
    const sink = recorder();
    const zoomed: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 400 };
    buildStrokePath(sink, zoomed, view, [[-170, 0], [170, 0]], 1);
    expect(sink.calls.filter((c) => c === "ellipse").length).toBeLessThanOrEqual(4000);
  });
});

describe("pixel sizes", () => {
  /**
   * The property the whole px option rests on: what the user asked for in
   * pixels is what the outline is drawn at. `kmFromPixels` resolves the ground
   * size and `footprintRadii` draws it, so the two must be exact inverses.
   */
  it("draws a pixel-valued brush at exactly that width", () => {
    for (const lat of [0, 30, -45, 60, 89]) {
      const km = kmFromPixels(camera, lat, 120);
      const { rx } = footprintRadii(camera, lat, km / 2);
      expect(rx * 2).toBeCloseTo(120, 6);
    }
  });

  it("round-trips back to kilometres", () => {
    for (const lat of [0, 30, -70]) {
      const px = pixelsFromKm(camera, lat, 850);
      expect(kmFromPixels(camera, lat, px)).toBeCloseTo(850, 6);
    }
  });

  /** The same pixel count buys less ground the further from the equator. */
  it("shrinks with latitude", () => {
    expect(kmFromPixels(camera, 60, 100)).toBeCloseTo(
      kmFromPixels(camera, 0, 100) / 2,
      4,
    );
  });

  it("keeps pixel brushes circular in globe views", () => {
    const globe = cameraForProjection({ centerLon: 0, centerLat: 0, pxPerDeg: 4 }, view, "orthographic");
    const km = kmFromPixels(globe, 45, 120, "projected", 0);
    const radii = footprintRadii(globe, 45, km / 2, "projected", 0);
    expect(radii.rx).toBeCloseTo(60, 6);
    expect(radii.ry).toBeCloseTo(60, 6);
    expect(pixelsFromKm(globe, 45, km, "projected", 0)).toBeCloseTo(120, 6);
  });

  /** Zooming in means fewer kilometres per pixel. */
  it("shrinks as the map zooms in", () => {
    const zoomed: Camera = { ...camera, pxPerDeg: camera.pxPerDeg * 4 };
    expect(kmFromPixels(zoomed, 0, 100)).toBeCloseTo(
      kmFromPixels(camera, 0, 100) / 4,
      6,
    );
  });

  it("stays finite at the pole", () => {
    expect(kmFromPixels(camera, 90, 100)).toBeGreaterThan(0);
    expect(Number.isFinite(pixelsFromKm(camera, 90, 100))).toBe(true);
  });
});

describe("pixel sizes in map space", () => {
  /**
   * What a size in pixels is actually asking for, and what a geodesic stamp
   * cannot give: that many pixels across *and* that many tall, at any
   * latitude (spec.md 3.5).
   */
  it("draws a pixel-valued brush as a circle of exactly that size", () => {
    for (const lat of [0, 30, -45, 60, 89]) {
      const km = kmFromPixels(camera, lat, 120, "projected");
      const { rx, ry } = footprintRadii(camera, lat, km / 2, "projected");
      expect(rx * 2).toBeCloseTo(120, 6);
      expect(ry * 2).toBeCloseTo(120, 6);
    }
  });

  /** No cosine anywhere, so the same pixels are the same kilometres anywhere. */
  it("does not shrink with latitude", () => {
    expect(kmFromPixels(camera, 70, 100, "projected")).toBeCloseTo(
      kmFromPixels(camera, 0, 100, "projected"),
      9,
    );
  });

  /** Still a screen measure: zooming in still means fewer kilometres per pixel. */
  it("shrinks as the map zooms in", () => {
    const zoomed: Camera = { ...camera, pxPerDeg: camera.pxPerDeg * 4 };
    expect(kmFromPixels(zoomed, 40, 100, "projected")).toBeCloseTo(
      kmFromPixels(camera, 40, 100, "projected") / 4,
      6,
    );
  });

  it("round-trips back to kilometres", () => {
    for (const lat of [0, 30, -70]) {
      const px = pixelsFromKm(camera, lat, 850, "projected");
      expect(kmFromPixels(camera, lat, px, "projected")).toBeCloseTo(850, 6);
    }
  });
});

describe("square footprints", () => {
  /**
   * `rect` opens its own subpath, so unlike `ellipse` a lone stamp needs no
   * `moveTo` — and adding one anyway would leave a stray point in the path.
   */
  it("draws one rect and nothing else for a single stamp", () => {
    const sink = recorder();
    buildStrokePath(sink, camera, view, [[0, 0]], 300, "square");
    expect(sink.calls).toEqual(["rect"]);
  });

  it("never reaches for an ellipse", () => {
    const sink = recorder();
    buildStrokePath(sink, camera, view, [[0, 0], [5, 5]], 300, "square");
    expect(sink.calls).not.toContain("ellipse");
    expect(sink.calls.filter((c) => c === "rect").length).toBeGreaterThan(1);
  });

  /**
   * The zig-zag (M58).
   *
   * A square stamp's union along a diagonal is a staircase: two axis-aligned
   * squares a few pixels apart meet only near their corners, so the edge is
   * serrated by the whole spacing rather than by the spacing squared. The
   * field has never had that edge — `swept_square_distance` measures to the
   * *segment* — so the preview was showing something the object was not.
   *
   * Checked against that definition rather than against the join's own
   * arithmetic: the swept region between two stamps is the convex hull of the
   * two squares, so every convex combination of a point in one and a point in
   * the other has to be covered by something the path drew.
   */
  it("covers the whole region the stamp sweeps between two samples", () => {
    const pieces = shapeRecorder();
    buildStrokePath(pieces.sink, camera, view, [[0, 0], [6, 5]], 300, "square");

    expect(pieces.rects.length).toBeGreaterThan(2);
    let checked = 0;
    let missedWithoutJoins = 0;
    for (let i = 1; i < pieces.rects.length; i++) {
      const a = pieces.rects[i - 1]!;
      const b = pieces.rects[i]!;
      for (const [ax, ay] of corners(a)) {
        for (const [bx, by] of corners(b)) {
          for (const s of [0.25, 0.5, 0.75]) {
            const x = ax + (bx - ax) * s;
            const y = ay + (by - ay) * s;
            checked += 1;
            expect(pieces.covers(x, y)).toBe(true);
            if (!pieces.rects.some((r) => inRect(r, x, y))) missedWithoutJoins += 1;
          }
        }
      }
    }
    expect(checked).toBeGreaterThan(100);
    // And the join is doing the work: the stamps alone leave those points out,
    // which is exactly the staircase that was on screen.
    expect(missedWithoutJoins).toBeGreaterThan(0);
  });

  /** The square is the bounding box of the circle of the same size. */
  it("spans the same screen extent as the round footprint", () => {
    const boxes: Array<{ x: number; y: number; w: number; h: number }> = [];
    const sink: PathSink = {
      moveTo: () => {},
      lineTo: () => {},
      closePath: () => {},
      ellipse: () => {},
      rect: (x, y, w, h) => boxes.push({ x, y, w, h }),
    };
    addFootprint(sink, camera, view, 10, 40, 300, "square");
    const { rx, ry } = footprintRadii(camera, 40, 300);
    expect(boxes).toHaveLength(1);
    expect(boxes[0]!.w).toBeCloseTo(rx * 2, 9);
    expect(boxes[0]!.h).toBeCloseTo(ry * 2, 9);
  });

  it("defaults to the round stamp", () => {
    const sink = recorder();
    addFootprint(sink, camera, view, 0, 0, 300);
    expect(sink.calls).toEqual(["moveTo", "ellipse"]);
  });
});

describe("footprintOfOutline", () => {
  /**
   * The backend's `radius_km` is the stamp's radius — the same half-extent a
   * `Footprint` carries — and it is passed through, not halved. Halving it drew
   * every outline at half the size of the object it outlined, which reads as an
   * outline sitting well inside the paint it belongs to.
   *
   * The independent reference is the backend's own construction: it sends
   * `radius_m * scale / 1000`, so a 400 km stamp at 100% arrives as 200.
   */
  it("keeps a swept outline's radius as the radius it is", () => {
    const [footprint] = footprintOfOutline({
      kind: "swept",
      chains: [[[0, 0] as const, [1, 0] as const]],
      radius_km: 200,
      square: false,
      space: "geodesic",
    });
    expect(footprint).toEqual({
      kind: "swept",
      points: [
        [0, 0],
        [1, 0],
      ],
      radiusKm: 200,
      shape: "circle",
      space: "geodesic",
    });
  });

  /** A stamp is a square by the same half-extent, measured across the flats. */
  it("carries the square stamp through", () => {
    const [footprint] = footprintOfOutline({
      kind: "swept",
      chains: [[[0, 0] as const]],
      radius_km: 50,
      square: true,
      space: "projected",
    });
    expect(footprint).toMatchObject({ shape: "square", space: "projected", radiusKm: 50 });
  });

  /** One footprint per chain: a merged object is several strokes in one. */
  it("gives a footprint per chain", () => {
    const outlines = footprintOfOutline({
      kind: "swept",
      chains: [[[0, 0] as const], [[5, 5] as const]],
      radius_km: 10,
      square: false,
      space: "geodesic",
    });
    expect(outlines).toHaveLength(2);
  });

  /** A closed ring is a polygon, which has no stamp and no radius. */
  it("turns a ring into a polygon", () => {
    expect(
      footprintOfOutline({
        kind: "ring",
        points: [
          [0, 0],
          [1, 0],
          [1, 1],
        ],
      }),
    ).toEqual([
      {
        kind: "polygon",
        points: [
          [0, 0],
          [1, 0],
          [1, 1],
        ],
      },
    ]);
  });
});

/** A rectangle as the sink reports it. */
interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

function inRect(box: Box, x: number, y: number): boolean {
  return (
    x >= box.x - 1e-9 &&
    x <= box.x + box.w + 1e-9 &&
    y >= box.y - 1e-9 &&
    y <= box.y + box.h + 1e-9
  );
}

/** A box's four corners, pulled a hair inside so a shared edge is not the test. */
function corners(box: Box): Array<[number, number]> {
  const inset = 1e-6;
  const x0 = box.x + inset;
  const x1 = box.x + box.w - inset;
  const y0 = box.y + inset;
  const y1 = box.y + box.h - inset;
  return [
    [x0, y0],
    [x1, y0],
    [x1, y1],
    [x0, y1],
  ];
}

/** Whether a point is inside a ring, by crossing number. */
function inPolygon(ring: ReadonlyArray<readonly [number, number]>, x: number, y: number): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i]!;
    const [xj, yj] = ring[j]!;
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/**
 * A sink that keeps the shapes rather than the call names, so a test can ask
 * what the path actually covers.
 */
function shapeRecorder(): {
  sink: PathSink;
  rects: Box[];
  rings: Array<Array<[number, number]>>;
  covers(x: number, y: number): boolean;
} {
  const rects: Box[] = [];
  const rings: Array<Array<[number, number]>> = [];
  let open: Array<[number, number]> = [];
  const sink: PathSink = {
    moveTo: (x, y) => {
      open = [[x, y]];
      rings.push(open);
    },
    lineTo: (x, y) => open.push([x, y]),
    closePath: () => {},
    ellipse: () => {},
    rect: (x, y, w, h) => rects.push({ x, y, w, h }),
  };
  return {
    sink,
    rects,
    rings,
    covers: (x, y) =>
      rects.some((box) => inRect(box, x, y)) || rings.some((ring) => inPolygon(ring, x, y)),
  };
}

it("keeps a 100 px tool circular in all projections at every latitude", () => {
  for (const { id: projection } of PROJECTIONS) {
    const space = projection === "equirectangular" ? "projected" : projection;
    for (const lat of [-75, 0, 60, 75]) {
      const camera = { centerLon: 0, centerLat: lat, pxPerDeg: 8, projection };
      const radiusKm = kmFromPixels(camera, lat, 50, space);
      const radius = footprintRadii(camera, lat, radiusKm, space);
      expect(radius.rx).toBeCloseTo(50, 9);
      expect(radius.ry).toBeCloseTo(50, 9);
    }
  }
});
