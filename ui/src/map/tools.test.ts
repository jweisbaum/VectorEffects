/**
 * The rules the option bar resolves.
 *
 * These are the frontend half of spec 6.1: which options a mode makes inert,
 * which gesture a tool is drawn with, what a size in px means, and what gets
 * frozen onto the object. The backend owns the *rules*; this owns applying them
 * to values only the bar holds, and that application is what is checked here.
 */
import { describe, expect, it } from "vitest";

import type { ToolOptionSpec } from "../generated/ToolOptionSpec";
import type { ToolSchema } from "../generated/ToolSchema";
import type { Camera } from "./camera";
import { footprintRadii, KM_PER_DEGREE } from "./footprint";
import { OP_POINTS } from "./renderer";
import {
  choiceOf,
  cloneSourceCamera,
  convertSizes,
  defaultState,
  eraserStamp,
  extentOf,
  flattenPath,
  footprintOf,
  frozenOptions,
  gestureKind,
  isLive,
  liveOptions,
  offersEyedropper,
  offersUnit,
  operatorOf,
  sampled,
  perimeterExtent,
  shownAngle,
  sizeKm,
  spaceFor,
  type ToolState,
} from "./tools";

const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 4 };

function option(over: Partial<ToolOptionSpec> & { property: string }): ToolOptionSpec {
  return {
    label: over.property,
    unit: "none",
    default: { kind: "number", value: 0 },
    min: null,
    max: null,
    variants: [],
    creation_only: false,
    slider: null,
    depends_on: [],
    ...over,
  };
}

/** A stand-in for the shape fill, whose two dependency axes cross. */
const shapeFill: ToolSchema = {
  tool: "shape_fill",
  label: "Shape fill",
  shortcut: "f",
  hover: false,
  preview: "field",
  gesture: {
    kind: "by_choice",
    on: "ShapeSource",
    gestures: ["ring", "extent", "extent", "extent"],
  },
  // Every shape it draws is drawn in a space, the freehand polygon included
  // (M57), so the unit is live in all four modes — the same rule
  // `stamp_space` carries, which the unit stands in for.
  sizing: { depends_on: [] },
  eyedropper: null,
  options: [
    option({
      property: "ShapeSource",
      default: { kind: "choice", index: 0 },
      variants: ["polygon", "square", "rectangle", "circle"],
      creation_only: true,
      slider: null,
    }),
    option({
      property: "VectorMode",
      default: { kind: "choice", index: 0 },
      variants: ["constant", "gradient"],
    }),
    option({
      property: "DirectionMode",
      default: { kind: "choice", index: 0 },
      variants: ["constant", "toward_point", "away_from_point"],
      depends_on: [{ on: "VectorMode", live_for: [0] }],
    }),
    option({
      property: "Speed",
      unit: "speed",
      default: { kind: "number", value: 10 },
      depends_on: [{ on: "VectorMode", live_for: [0] }],
    }),
    option({
      property: "Direction",
      unit: "direction",
      default: { kind: "angle", degrees: 0 },
      depends_on: [
        { on: "VectorMode", live_for: [0] },
        { on: "DirectionMode", live_for: [0] },
      ],
    }),
    option({
      property: "Target",
      default: { kind: "position", lon: 0, lat: 0 },
      depends_on: [
        { on: "VectorMode", live_for: [0] },
        { on: "DirectionMode", live_for: [1, 2] },
      ],
    }),
    option({
      property: "SpeedStart",
      unit: "speed",
      default: { kind: "number", value: 5 },
      depends_on: [{ on: "VectorMode", live_for: [1] }],
    }),
  ],
};

/** A stand-in for the brush: one size, one space, one gesture. */
const brush: ToolSchema = {
  tool: "brush",
  label: "Brush",
  shortcut: "b",
  hover: true,
  preview: "field",
  gesture: { kind: "always", gesture: "stroke" },
  sizing: { depends_on: [] },
  eyedropper: null,
  options: [
    option({
      property: "BrushShape",
      default: { kind: "choice", index: 0 },
      variants: ["circle", "square"],
      creation_only: true,
      slider: null,
    }),
    option({
      property: "SizeKm",
      unit: "kilometres",
      default: { kind: "number", value: 500 },
    }),
    option({ property: "Speed", unit: "speed", default: { kind: "number", value: 10 } }),
  ],
};

describe("isLive", () => {
  /**
   * The shape fill crosses two axes: `direction` is read only when the object
   * is *both* on a constant vector and on a fixed bearing. Rules on one option
   * must therefore and together — a version that ored them would show the
   * bearing on a gradient, where nothing reads it.
   */
  it("requires every rule on an option to hold", () => {
    const direction = shapeFill.options.find((o) => o.property === "Direction")!;

    const constant = { VectorMode: { kind: "choice" as const, index: 0 }, DirectionMode: { kind: "choice" as const, index: 0 } };
    expect(isLive(direction, constant)).toBe(true);

    // Constant vector, but aimed at a point: the bearing is not read.
    expect(
      isLive(direction, { ...constant, DirectionMode: { kind: "choice", index: 1 } }),
    ).toBe(false);

    // Gradient: neither the bearing nor the aim mode is read.
    expect(
      isLive(direction, { ...constant, VectorMode: { kind: "choice", index: 1 } }),
    ).toBe(false);
  });

  it("treats an option with no rules as always live", () => {
    expect(isLive(option({ property: "Feather" }), {})).toBe(true);
  });

  /** A missing value reads as variant 0, which is the schema default. */
  it("falls back to the first variant when a mode is unset", () => {
    const speed = shapeFill.options.find((o) => o.property === "Speed")!;
    expect(isLive(speed, {})).toBe(true);
  });
});

describe("liveOptions", () => {
  /**
   * Switching to the gradient must swap which half of the panel is shown, not
   * add to it: an inert option left on screen invites editing a value and
   * watching nothing happen.
   */
  it("swaps the constant options for the gradient ones", () => {
    const state = defaultState(shapeFill);
    const constant = liveOptions(shapeFill, state.values).map((o) => o.property);
    expect(constant).toContain("Speed");
    expect(constant).toContain("Direction");
    expect(constant).not.toContain("SpeedStart");

    const gradient = liveOptions(shapeFill, {
      ...state.values,
      VectorMode: { kind: "choice", index: 1 },
    }).map((o) => o.property);
    expect(gradient).toContain("SpeedStart");
    expect(gradient).not.toContain("Speed");
    expect(gradient).not.toContain("Direction");
    expect(gradient).not.toContain("Target");
  });

  /** No mode may empty the bar; there is always something left to set. */
  it("leaves something editable in every mode", () => {
    for (const vector of [0, 1]) {
      for (const aim of [0, 1, 2]) {
        const live = liveOptions(shapeFill, {
          VectorMode: { kind: "choice", index: vector },
          DirectionMode: { kind: "choice", index: aim },
        });
        expect(live.length).toBeGreaterThan(0);
      }
    }
  });
});

describe("gestureKind", () => {
  it("takes the tool's single gesture when it has one", () => {
    expect(gestureKind(brush, {})).toBe("stroke");
  });

  /**
   * The shape fill's gesture is its `shape_source`: a polygon is placed vertex
   * by vertex and a preset is dragged out. Sending the wrong one is refused by
   * the backend, so this is the mapping that keeps that from happening.
   */
  it("follows the shape fill's own source", () => {
    expect(gestureKind(shapeFill, { ShapeSource: { kind: "choice", index: 0 } })).toBe("ring");
    for (const preset of [1, 2, 3]) {
      expect(gestureKind(shapeFill, { ShapeSource: { kind: "choice", index: preset } })).toBe(
        "extent",
      );
    }
  });

  it("falls back to the first gesture for a variant it does not know", () => {
    expect(gestureKind(shapeFill, { ShapeSource: { kind: "choice", index: 9 } })).toBe("ring");
  });
});

describe("offersUnit", () => {
  /**
   * The unit is the stamp-space control, so a tool must never offer both —
   * two controls for one property, and the space would be the one that did
   * nothing. The backend leaves the space out of the options; this checks the
   * bar does not put it back.
   */
  it("is the only control for the space", () => {
    for (const schema of [brush, shapeFill]) {
      expect(schema.options.some((o) => o.property === "StampSpace")).toBe(false);
    }
  });

  it("is offered by a tool that measures", () => {
    expect(offersUnit(brush, defaultState(brush).values)).toBe(true);
  });

  it("is not offered by a tool that measures nothing", () => {
    expect(offersUnit({ ...brush, sizing: null }, {})).toBe(false);
  });

  /**
   * The shape fill offers it in all four of its modes, the freehand polygon
   * included (M57): the space is the plane the shape is drawn in, not only a
   * unit for a typed number, and a polygon has a plane like anything else.
   */
  it("follows the same rule the space it selects follows", () => {
    const source = (index: number) => ({ ShapeSource: { kind: "choice" as const, index } });
    for (const shape of [0, 1, 2, 3]) {
      expect(offersUnit(shapeFill, source(shape))).toBe(true);
    }
  });
});

describe("the eraser's stamp", () => {
  /**
   * The report (M67): the eraser's px distorted going north. It had no stamp
   * space at all — every one of its footprints said "geodesic" and its size
   * converted through `cos(lat)` — so px was a bare unit conversion, which is
   * the one thing spec 3.5 says the unit is not.
   *
   * Checked in the pixels it actually draws, not in the kilometres it stores:
   * a stamp asked for in pixels has to *be* that many pixels, both ways,
   * wherever it is used.
   */
  it("draws the pixels it was asked for, at every latitude", () => {
    for (const lat of [0, 30, 45, 60, 70, -55]) {
      const { radiusKm, space } = eraserStamp({ size: 80, unit: "px" }, camera, lat);
      const { rx, ry } = footprintRadii(camera, lat, radiusKm, space);
      expect(rx * 2, `${lat} wide`).toBeCloseTo(80, 6);
      expect(ry * 2, `${lat} tall`).toBeCloseTo(80, 6);
    }
  });

  /** And km is still a ground size: the same kilometres wherever it is used. */
  it("keeps a size in km on the ground", () => {
    for (const lat of [0, 45, 70]) {
      const { radiusKm, space } = eraserStamp({ size: 600, unit: "km" }, camera, lat);
      expect(radiusKm).toBe(300);
      expect(space).toBe("geodesic");
    }
  });

  /** The unit is the space, here as everywhere else. */
  it("takes its space from its unit", () => {
    expect(eraserStamp({ size: 80, unit: "px" }, camera, 40).space).toBe("projected");
    expect(eraserStamp({ size: 80, unit: "km" }, camera, 40).space).toBe("geodesic");
  });
});

describe("sizes in px and km", () => {
  /** px is a shape on the map, km one on the ground (spec.md 3.5). */
  it("selects the stamp space from the unit", () => {
    expect(spaceFor("px")).toBe("projected");
    expect(spaceFor("km")).toBe("geodesic");
  });

  it("passes a km size through untouched at any latitude", () => {
    const state: ToolState = { values: { SizeKm: { kind: "number", value: 400 } }, unit: "km" };
    for (const lat of [0, 45, 70]) {
      expect(sizeKm(state, "SizeKm", camera, lat)).toBe(400);
    }
  });

  /**
   * A projected stamp is the same number of pixels tall at any latitude, so the
   * ground size a px number resolves to is the same everywhere — which is what
   * makes it a shape on the map.
   */
  it("resolves a px size to the same ground height at every latitude", () => {
    const state: ToolState = { values: { SizeKm: { kind: "number", value: 80 } }, unit: "px" };
    const expected = (80 / camera.pxPerDeg) * KM_PER_DEGREE;
    for (const lat of [0, 45, 70]) {
      expect(sizeKm(state, "SizeKm", camera, lat)).toBeCloseTo(expected, 6);
    }
  });

  /**
   * Switching the unit must not resize the tool.
   *
   * The px field holds whole pixels, so a round trip can lose up to half of one
   * in each direction — that is the whole of the permitted drift, and it is
   * stated in pixels rather than as a percentage because that is what it is.
   * Converting in the wrong space instead costs a factor of `cos(lat)`, which
   * is a fifth of the size at 40° and nothing like a rounding error.
   */
  it("carries the size across a change of unit", () => {
    const onePixelKm = KM_PER_DEGREE / camera.pxPerDeg;
    const start: ToolState = { values: { SizeKm: { kind: "number", value: 600 } }, unit: "km" };
    const asPixels = convertSizes(start, brush, "px", camera, 40);
    const andBack = convertSizes(asPixels, brush, "km", camera, 40);

    const original = sizeKm(start, "SizeKm", camera, 40);
    const returned = sizeKm(andBack, "SizeKm", camera, 40);
    expect(Math.abs(returned - original)).toBeLessThanOrEqual(onePixelKm);

    // ...and the drift really is rounding, not a scale factor: a wrong space
    // would be out by cos(40°), which is far more than a pixel here.
    const wrong = original * Math.cos((40 * Math.PI) / 180);
    expect(Math.abs(returned - wrong)).toBeGreaterThan(onePixelKm);
  });

  it("leaves the state alone when the unit does not change", () => {
    const start = defaultState(brush);
    expect(convertSizes(start, brush, "km", camera, 0)).toBe(start);
  });
});

describe("frozenOptions", () => {
  /**
   * The unit chose the space, so the space must reach the object — otherwise a
   * px stroke is stored as a ground shape and paints an ellipse.
   */
  it("sends the stamp space the unit selected", () => {
    const state: ToolState = { values: { SizeKm: { kind: "number", value: 80 } }, unit: "px" };
    const sent = frozenOptions(state, brush, camera, 60);
    expect(sent).toContainEqual({
      property: "StampSpace",
      value: { kind: "choice", index: 1 },
    });
  });

  it("sends sizes in kilometres, never in pixels", () => {
    const state: ToolState = { values: { SizeKm: { kind: "number", value: 80 } }, unit: "px" };
    const sent = frozenOptions(state, brush, camera, 0);
    const size = sent.find((o) => o.property === "SizeKm")!;
    expect(size.value).toEqual({
      kind: "number",
      value: (80 / camera.pxPerDeg) * KM_PER_DEGREE,
    });
  });

  /**
   * Hidden is not deleted (spec.md 6.1): an inert option keeps its value on the
   * object, so switching the mode back brings it back as it was.
   */
  it("freezes the inert options too", () => {
    const state = defaultState(shapeFill);
    const sent = frozenOptions(
      { ...state, values: { ...state.values, VectorMode: { kind: "choice", index: 1 } } },
      shapeFill,
      camera,
      0,
    ).map((o) => o.property);
    expect(sent).toContain("Direction");
    expect(sent).toContain("Target");
  });

  /**
   * A tool that measures anything sends the space its unit selected, even for
   * a mode that does not read it — hidden is not deleted, and the object still
   * carries the property at a known value (spec.md 6.1).
   */
  it("sends the space for a tool that measures, whatever its mode", () => {
    const sent = frozenOptions(defaultState(shapeFill), shapeFill, camera, 0);
    expect(sent).toContainEqual({
      property: "StampSpace",
      value: { kind: "choice", index: 0 },
    });
  });

  /** A tool that measures nothing has no space to send. */
  it("sends no stamp space for a tool that measures nothing", () => {
    const measureless: ToolSchema = { ...shapeFill, sizing: null };
    const sent = frozenOptions(defaultState(measureless), measureless, camera, 0);
    expect(sent.some((o) => o.property === "StampSpace")).toBe(false);
  });
});

describe("shownAngle", () => {
  /**
   * A flow direction is shown in the project's convention; a geometric bearing
   * is not. Getting this wrong shows the reciprocal of what the user set.
   */
  it("converts a flow direction and leaves a bearing alone", () => {
    expect(shownAngle("direction", "from", 270)).toBe(90);
    expect(shownAngle("degrees", "from", 270)).toBe(270);
    expect(shownAngle("direction", "toward", 270)).toBe(270);
  });

  it("is its own inverse", () => {
    for (const degrees of [0, 45, 180, 359]) {
      expect(shownAngle("direction", "from", shownAngle("direction", "from", degrees))).toBe(
        degrees,
      );
    }
  });
});

describe("footprintOf", () => {
  const state = (values: Record<string, number>, unit: "km" | "px" = "km"): ToolState => ({
    values: Object.fromEntries(
      Object.entries(values).map(([k, v]) => [k, { kind: "number" as const, value: v }]),
    ),
    unit,
  });

  it("sweeps the brush's stamp along its stroke", () => {
    const footprint = footprintOf(
      "brush",
      state({ SizeKm: 400 }),
      { kind: "stroke", points: [[0, 0], [2, 0]] },
      camera,
    );
    expect(footprint).toEqual({
      kind: "swept",
      points: [[0, 0], [2, 0]],
      // The size is a diameter, so the footprint's radius is half of it.
      radiusKm: 200,
      shape: "circle",
      space: "geodesic",
    });
  });

  /** Fill mode 1 is the ring, and a ring is not a disc. */
  it("previews a perimeter circle as a ring and the others as discs", () => {
    const base = { DiameterKm: 1000, RingWidthKm: 200 };
    const disc = footprintOf("circle", state(base), { kind: "point", at: [0, 0] }, camera);
    expect(disc?.kind).toBe("disc");

    const ring = footprintOf(
      "circle",
      {
        values: {
          DiameterKm: { kind: "number", value: 1000 },
          RingWidthKm: { kind: "number", value: 200 },
          FillMode: { kind: "choice", index: 1 },
        },
        unit: "km",
      },
      { kind: "point", at: [0, 0] },
      camera,
    );
    expect(ring).toMatchObject({ kind: "ring", radiusKm: 500, halfWidthKm: 100 });
  });

  /** A square preset takes the larger reach, so a wide drag makes a square. */
  it("reads one drag as each of the three presets", () => {
    const drag = { kind: "extent" as const, centre: [0, 0] as [number, number], rim: [4, 1] as [number, number] };
    const source = (index: number): ToolState => ({
      values: { ShapeSource: { kind: "choice", index } },
      unit: "km",
    });

    const square = footprintOf("shape_fill", source(1), drag, camera);
    expect(square).toMatchObject({ kind: "rect" });
    expect((square as { halfWidthKm: number }).halfWidthKm).toBeCloseTo(
      (square as { halfHeightKm: number }).halfHeightKm,
      6,
    );

    const rect = footprintOf("shape_fill", source(2), drag, camera);
    expect((rect as { halfWidthKm: number }).halfWidthKm).toBeGreaterThan(
      (rect as { halfHeightKm: number }).halfHeightKm,
    );

    expect(footprintOf("shape_fill", source(3), drag, camera)?.kind).toBe("disc");
  });

  it("has nothing to draw until a polygon has an inside", () => {
    const ring = (points: Array<[number, number]>) =>
      footprintOf("shape_fill", defaultState(shapeFill), { kind: "ring", points }, camera);
    expect(ring([[0, 0]])).toBeNull();
    expect(ring([[0, 0], [1, 0]])).toBeNull();
    expect(ring([[0, 0], [1, 0], [1, 1]])?.kind).toBe("polygon");
  });

  it("has nothing to draw for a one-node path or a drag that has not moved", () => {
    expect(
      footprintOf(
        "curve",
        state({ WidthKm: 300 }),
        { kind: "path", nodes: [{ at: [0, 0] }] },
        camera,
      ),
    ).toBeNull();
    expect(
      footprintOf(
        "shape_fill",
        defaultState(shapeFill),
        { kind: "extent", centre: [0, 0], rim: [0, 0] },
        camera,
      ),
    ).toBeNull();
  });

  /** A curve is a corridor swept along its path, half the width either side. */
  it("previews a curve as a corridor along its path", () => {
    const footprint = footprintOf(
      "curve",
      state({ WidthKm: 300 }),
      { kind: "path", nodes: [{ at: [0, 0] }, { at: [4, 0] }] },
      camera,
    );
    expect(footprint).toMatchObject({ kind: "swept", radiusKm: 150 });
  });
});

describe("extentOf", () => {
  /**
   * A geodesic drag's east-west reach is a ground distance, so the same drag
   * in degrees covers less ground the further north it is; a projected drag is
   * measured on the map and does not shrink.
   */
  it("measures a drag on the ground or on the map, as the space says", () => {
    const drag = ([0, 60] as const);
    const ground = extentOf(drag, [4, 60], "geodesic");
    const map = extentOf(drag, [4, 60], "projected");

    expect(map.halfWidthKm).toBeCloseTo(4 * KM_PER_DEGREE, 6);
    expect(ground.halfWidthKm).toBeCloseTo(4 * KM_PER_DEGREE * Math.cos((60 * Math.PI) / 180), 1);
  });

  /** A drag across the dateline is a drag, not a lap of the world. */
  it("takes the shorter way round", () => {
    const across = extentOf([179, 0], [-179, 0], "projected");
    expect(across.halfWidthKm).toBeCloseTo(2 * KM_PER_DEGREE, 6);
  });
});

describe("flattenPath", () => {
  it("leaves a handleless path as its own nodes", () => {
    expect(
      flattenPath([{ at: [0, 0] }, { at: [1, 1] }, { at: [2, 0] }]),
    ).toEqual([[0, 0], [1, 1], [2, 0]]);
  });

  /**
   * A Bézier must actually leave the chord. A flattening that ignored the
   * handles would return the two endpoints and look exactly like a polyline.
   */
  it("bends a segment away from its chord", () => {
    const points = flattenPath([
      { at: [-4, 0], out_handle: [-2, 6] },
      { at: [4, 0], in_handle: [2, 6] },
    ]);
    expect(points.length).toBeGreaterThan(10);
    const highest = Math.max(...points.map(([, lat]) => lat));
    expect(highest).toBeGreaterThan(2);
    // The ends are still the nodes themselves.
    expect(points[0]).toEqual([-4, 0]);
    expect(points[points.length - 1]![0]).toBeCloseTo(4, 6);
  });

  it("returns nothing for an empty path", () => {
    expect(flattenPath([])).toEqual([]);
  });
});

describe("choiceOf", () => {
  it("reads a choice and defaults to the first variant otherwise", () => {
    expect(choiceOf({ Mode: { kind: "choice", index: 2 } }, "Mode")).toBe(2);
    expect(choiceOf({}, "Mode")).toBe(0);
    expect(choiceOf({ Mode: { kind: "number", value: 3 } }, "Mode")).toBe(0);
  });
});

describe("operatorOf", () => {
  const view = { width: 1000, height: 500 };
  const at = (lon: number, lat: number): [number, number] => [lon, lat];
  const stroke = (...points: [number, number][]) => ({ kind: "stroke" as const, points });
  const state = (values: ToolState["values"]): ToolState => ({ values, unit: "km" });

  /** Spec 6.3: the intensity's amount is a percentage; the kernel takes a fraction. */
  it("carries an intensity's gain as a fraction", () => {
    const spec = operatorOf(
      "intensity",
      state({ Gain: { kind: "number", value: 50 } }),
      stroke(at(0, 0)),
      camera,
      view,
    );
    expect(spec).toEqual({ kind: "gain", amount: 0.5 });
  });

  /** M29: sense 0 is clockwise, the positive turn; sense 1 is the other way. */
  it("signs a turn by its sense", () => {
    const turn = (sense: number) =>
      operatorOf(
        "turn",
        state({
          TurnAmountDeg: { kind: "number", value: 30 },
          TurnSense: { kind: "choice", index: sense },
        }),
        stroke(at(0, 0)),
        camera,
        view,
      );
    expect(turn(0)).toEqual({ kind: "turn", amount: 30 });
    expect(turn(1)).toEqual({ kind: "turn", amount: -30 });
  });

  /**
   * The stroke's points reach the shader in framebuffer pixels with y up,
   * as gl_FragCoord measures them: a point north of the centre is above it.
   */
  it("gives a divergence its centreline in framebuffer pixels, y up", () => {
    const spec = operatorOf(
      "divergence",
      state({ Radial: { kind: "number", value: 100 } }),
      stroke(at(camera.centerLon, camera.centerLat), at(camera.centerLon, camera.centerLat + 1)),
      camera,
      view,
    );
    expect(spec?.kind).toBe("radial");
    expect(spec?.amount).toBe(1);
    expect(spec?.points?.[0]).toEqual([500, 250]);
    expect(spec?.points?.[1]?.[1]).toBeGreaterThan(250);
  });

  /**
   * A liquify's stamps carry the pointer's movement, scaled by the strength
   * (spec.md 6.3); thinning a long stroke keeps the whole movement, so the
   * field is dragged as far by the preview as by the commit.
   */
  it("gives a liquify each stamp's movement and keeps it through thinning", () => {
    const points: [number, number][] = [];
    for (let i = 0; i <= 200; i++) points.push(at(camera.centerLon + i * 0.01, camera.centerLat));
    const spec = operatorOf(
      "liquify",
      state({
        Strength: { kind: "number", value: 50 },
        SizeKm: { kind: "number", value: 100 },
        Feather: { kind: "number", value: 0.5 },
      }),
      stroke(...points),
      camera,
      view,
    );
    expect(spec?.kind).toBe("smear");
    expect(spec?.points?.length).toBe(OP_POINTS);
    expect(spec?.deltas?.[0]).toEqual([0, 0]);
    const whole = (spec?.deltas ?? []).reduce((sum, [dx]) => sum + dx, 0);
    // 2 degrees of travel at the camera's scale, halved by the strength.
    expect(whole).toBeCloseTo(2 * camera.pxPerDeg * 0.5, 6);
    expect(spec?.radiusPx).toBeGreaterThan(0);
    expect(spec?.feather).toBe(0.5);
  });

  /**
   * A warp's push is a clone of the field under the start, read through a
   * camera shifted so it lands at the end (M29: the push is the stroke); a
   * twist has no screen-space preview, and neither has a click that went
   * nowhere.
   */
  it("previews a push as a shifted read and a twist not at all", () => {
    const push = operatorOf(
      "warp",
      state({ WarpMode: { kind: "choice", index: 0 } }),
      stroke(at(10, 20), at(15, 20)),
      camera,
      view,
    );
    expect(push?.kind).toBe("clone");
    expect(push?.source?.centerLon).toBeCloseTo(camera.centerLon - 5, 9);
    expect(
      operatorOf("warp", state({ WarpMode: { kind: "choice", index: 1 } }), stroke(at(10, 20), at(15, 20)), camera, view),
    ).toBeNull();
    expect(
      operatorOf("warp", state({ WarpMode: { kind: "choice", index: 0 } }), stroke(at(10, 20)), camera, view),
    ).toBeNull();
  });

  /** A tool that paints a field of its own is the overlay's to preview. */
  it("is nothing for a tool that paints", () => {
    expect(operatorOf("brush", state({}), stroke(at(0, 0)), camera, view)).toBeNull();
  });
});

describe("cloneSourceCamera", () => {
  const state = (source: [number, number], mode = 0): ToolState => ({
    values: {
      SourcePoint: { kind: "position", lon: source[0], lat: source[1] },
      OffsetMode: { kind: "choice", index: mode },
    },
    unit: "km",
  });

  /**
   * The camera the source is read through is the map's own, shifted so that
   * what is at the source lands where the brush is. Shifted the *other* way and
   * the preview would show a patch from twice the offset away, which looks
   * plausible and is wrong.
   */
  it("shifts the camera so the source lands under the brush", () => {
    const gesture = { kind: "stroke" as const, points: [[10, 20] as [number, number]] };
    const source = cloneSourceCamera(state([40, 5]), gesture, camera);

    // The brush is at 10E 20N and reads from 40E 5N, so the offset is
    // -30 degrees of longitude and +15 of latitude.
    expect(source).toEqual({
      centerLon: camera.centerLon - (10 - 40),
      centerLat: camera.centerLat - (20 - 5),
      pxPerDeg: camera.pxPerDeg,
    });
  });

  /** The zoom is the map's: a clone copies the field, not a magnification. */
  it("keeps the map's own scale", () => {
    const gesture = { kind: "stroke" as const, points: [[0, 0] as [number, number]] };
    expect(cloneSourceCamera(state([40, 0]), gesture, camera)?.pxPerDeg).toBe(camera.pxPerDeg);
  });

  /**
   * `Aligned` measures the offset from where the gesture *began*, so the source
   * travels with the brush and the whole stroke reads one continuous band.
   * `Fixed` measures it from where the pointer is now, so the source stays put
   * — exact at the head of the stroke, which is where the user is looking.
   */
  it("measures the offset from the anchor when aligned and the pointer when fixed", () => {
    const gesture = {
      kind: "stroke" as const,
      points: [[0, 0], [20, 0]] as Array<[number, number]>,
    };

    const aligned = cloneSourceCamera(state([40, 0], 0), gesture, camera);
    expect(aligned?.centerLon).toBeCloseTo(camera.centerLon - (0 - 40), 9);

    const fixed = cloneSourceCamera(state([40, 0], 1), gesture, camera);
    expect(fixed?.centerLon).toBeCloseTo(camera.centerLon - (20 - 40), 9);
  });

  /**
   * An offset across the dateline is the short way, like every other one: a
   * brush at 179°W reading from 179°E is two degrees along, not 358.
   *
   * Asserted as the property the shift exists for — the source point lands
   * where the brush is — rather than as a centre value, because the map draws a
   * copy of the world every 360° and the two are the same place.
   */
  it("takes the shorter way round the dateline", () => {
    const brush: [number, number] = [-179, 0];
    const source: [number, number] = [179, 0];
    const shifted = cloneSourceCamera(state(source), {
      kind: "stroke",
      points: [brush],
    }, camera);
    expect(shifted).not.toBeNull();

    // Where the source sits under the shifted camera is where the brush sits
    // under the real one.
    const under = (lon: number, centre: number) => {
      const delta = ((lon - centre + 540) % 360) - 180;
      return delta;
    };
    expect(under(source[0], shifted!.centerLon)).toBeCloseTo(
      under(brush[0], camera.centerLon),
      9,
    );

    // ...and the shift really was two degrees, not the long way round.
    expect(Math.abs(shifted!.centerLon - camera.centerLon)).toBeCloseTo(2, 9);
  });

  /** Only a stroke clones, and only once it has a point to clone from. */
  it("has no answer for a gesture that is not a stroke", () => {
    expect(cloneSourceCamera(state([40, 0]), { kind: "point", at: [0, 0] }, camera)).toBeNull();
    expect(
      cloneSourceCamera(state([40, 0]), { kind: "stroke", points: [] }, camera),
    ).toBeNull();
  });
});

describe("offersEyedropper", () => {
  /**
   * Spec 6.1: the eyedropper writes one speed and one bearing, so it is offered
   * exactly where the tool paints one of each. The shape fill's gradient mode
   * has two of each, and no single answer to give.
   */
  it("follows the same dependency rules as the options it writes", () => {
    const withDropper: ToolSchema = {
      ...shapeFill,
      eyedropper: {
        speed: "Speed",
        direction: "Direction",
        depends_on: [
          { on: "VectorMode", live_for: [0] },
          { on: "DirectionMode", live_for: [0] },
        ],
      },
    };
    const values = (vector: number, direction: number) => ({
      VectorMode: { kind: "choice" as const, index: vector },
      DirectionMode: { kind: "choice" as const, index: direction },
    });
    expect(offersEyedropper(withDropper, values(0, 0))).toBe(true);
    expect(offersEyedropper(withDropper, values(1, 0))).toBe(false);
    expect(offersEyedropper(withDropper, values(0, 1))).toBe(false);
  });

  it("is not offered at all by a tool that declares none", () => {
    expect(offersEyedropper(shapeFill, {})).toBe(false);
  });
});

describe("sampled", () => {
  /**
   * A sample is written in stored units — m/s and an azimuth-toward — so it
   * takes exactly the path a typed number takes and is converted for display
   * once, in the bar (spec.md 3.3). Writing knots here would paint a stroke
   * nearly twice as fast as the one it was taken from.
   */
  it("writes the speed and the bearing as the backend reported them", () => {
    const schema: ToolSchema = {
      ...shapeFill,
      eyedropper: { speed: "Speed", direction: "Direction", depends_on: [] },
    };
    const state: ToolState = { values: { Feather: { kind: "number", value: 0.5 } }, unit: "km" };
    const next = sampled(state, schema, { speed_mps: 12.5, azimuth_toward_deg: 275.5 });

    expect(next.values.Speed).toEqual({ kind: "number", value: 12.5 });
    expect(next.values.Direction).toEqual({ kind: "angle", degrees: 275.5 });
    expect(next.values.Feather, "everything else is left alone").toEqual({
      kind: "number",
      value: 0.5,
    });
    expect(next.unit).toBe("km");
  });

  it("leaves a tool with no eyedropper untouched", () => {
    const state: ToolState = { values: {}, unit: "px" };
    expect(sampled(state, shapeFill, { speed_mps: 9, azimuth_toward_deg: 90 })).toBe(state);
  });

  /**
   * Pointing where nothing is written takes nothing (M53). An undefined
   * sample arrives as zero, and writing it would set the tool to a dead calm
   * due north — a number the user did not point at, and would have to notice
   * before undoing.
   */
  it("takes nothing from a point with no field", () => {
    const schema: ToolSchema = {
      ...shapeFill,
      eyedropper: { speed: "Speed", direction: "Direction", depends_on: [] },
    };
    const state: ToolState = { values: {}, unit: "km" };
    const empty = sampled(state, schema, {
      speed_mps: 0,
      azimuth_toward_deg: 0,
      defined: false,
    });
    expect(empty).toBe(state);
    // And a defined sample of a genuine calm is still taken: zero is a value.
    const calm = sampled(state, schema, {
      speed_mps: 0,
      azimuth_toward_deg: 0,
      defined: true,
    });
    expect(calm).not.toBe(state);
  });
});

describe("perimeterExtent", () => {
  /**
   * The press is on the perimeter and the release opposite it (M56), so the
   * shape sits between the pointer's two positions. Growing it from the press
   * as a centre put the click in the middle of the result, and where its edge
   * would land was a guess.
   */
  it("puts the press on the perimeter and the shape between the two points", () => {
    const drawn = perimeterExtent([0, 0], [4, -2], "rect", "projected");
    expect(drawn.centre[0]).toBeCloseTo(2, 12);
    expect(drawn.centre[1]).toBeCloseTo(-1, 12);
    // Half the drag each way, so the press is a corner and the release the
    // opposite one.
    expect(drawn.halfWidthKm).toBeCloseTo(2 * KM_PER_DEGREE, 6);
    expect(drawn.halfHeightKm).toBeCloseTo(1 * KM_PER_DEGREE, 6);
  });

  /** A circle's drag is a diameter: the centre is halfway along it. */
  it("makes the drag a diameter for a circle", () => {
    const drawn = perimeterExtent([0, 0], [0, -6], "circle", "projected");
    expect(drawn.centre).toEqual([0, -3]);
    expect(drawn.halfHeightKm).toBeCloseTo(3 * KM_PER_DEGREE, 6);
  });

  /**
   * A square takes the larger reach, so a mostly sideways drag makes the
   * square it looks like it is making — with the press still on a corner.
   */
  it("squares to the larger reach, keeping the press on a corner", () => {
    const drawn = perimeterExtent([0, 0], [6, -1], "square", "projected");
    expect(drawn.halfWidthKm).toBeCloseTo(drawn.halfHeightKm, 9);
    expect(drawn.halfWidthKm).toBeCloseTo(3 * KM_PER_DEGREE, 6);
    // The centre is half a side from the press, in the direction dragged.
    expect(drawn.centre[0]).toBeCloseTo(3, 6);
    expect(drawn.centre[1]).toBeCloseTo(-3, 6);
  });

  /** Dragged the other way, the shape goes the other way. */
  it("follows the direction of the drag", () => {
    const up = perimeterExtent([0, 0], [-4, 2], "rect", "projected");
    expect(up.centre[0]).toBeCloseTo(-2, 12);
    expect(up.centre[1]).toBeCloseTo(1, 12);
    expect(up.halfWidthKm).toBeGreaterThan(0);
  });

  /** A drag across the dateline is a drag, not a trip round the world. */
  it("takes the shorter way round the seam", () => {
    const across = perimeterExtent([179, 0], [-179, 0], "rect", "projected");
    expect(across.halfWidthKm).toBeCloseTo(KM_PER_DEGREE, 6);
    expect(across.centre[0]).toBeCloseTo(180, 9);
  });
});
