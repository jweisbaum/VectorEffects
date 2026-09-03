/**
 * The gesture state machine.
 *
 * What a press, a move and a release do, for each of the five gestures. These
 * are the rules a user feels rather than reads, and they were the part of the
 * tools with no test at all — the component around them needs a canvas, a
 * camera and a pointer, so none of it could be checked without all three.
 *
 * Everything here is keyed on the gesture and never on the tool, which is the
 * property that makes two tools drawing the same way behave the same way.
 */
import { describe, expect, it } from "vitest";

import type { ToolSchema } from "../generated/ToolSchema";
import {
  dragPixels,
  finished,
  gestureLatitude,
  hoverGesture,
  type InProgress,
  isComplete,
  MIN_DRAG_PX,
  press,
  release,
  shapeNode,
} from "./gesture";
import type { ToolState } from "./tools";

/** The map scale the tests measure drags against: 4 px per degree. */
const pxPerDeg = 4;

function at(lon: number, lat: number): [number, number] {
  return [lon, lat];
}

describe("the shape fill's presets", () => {
  /**
   * Square, rectangle and circle are **one press-drag-release gesture**, not
   * three interactions. They differ only in how the drag is read into a
   * footprint, which happens where the footprint is built — so there is nothing
   * per-preset here, and that is the point: a preset cannot end up with its own
   * interaction by accident.
   */
  it("are all begun by a press and finished by the release", () => {
    // The press begins an extent whose centre and rim are both where it landed.
    const begun = press("extent", null, at(10, 20), false);
    expect(begun).toEqual({
      act: "draw",
      drawing: { kind: "extent", centre: [10, 20], rim: [10, 20] },
      shapeHandles: false,
    });

    // ...the pointer drags the rim out...
    const drawing = (begun as { drawing: InProgress }).drawing as Extract<
      InProgress,
      { kind: "extent" }
    >;
    drawing.rim = at(14, 22);

    // ...and the release finishes it, rather than holding it for more clicks.
    expect(release(drawing)).toBe("finish");
    expect(finished(drawing)).toEqual({
      kind: "extent",
      centre: [10, 20],
      rim: [14, 22],
    });
  });

  /**
   * A press and release at one point describes a shape of no size. Committing
   * it would put an invisible object in the document, so it is no gesture at
   * all — and neither is a press with a pixel of hand tremor, which is the
   * case that actually happens.
   */
  it("are not committed by a click that does not drag", () => {
    const clicked: InProgress = { kind: "extent", centre: at(10, 20), rim: at(10, 20) };
    expect(isComplete(clicked, pxPerDeg)).toBe(false);

    // One pixel of tremor: still a click, not a drag.
    const tremor: InProgress = {
      kind: "extent",
      centre: at(10, 20),
      rim: at(10 + 1 / pxPerDeg, 20),
    };
    expect(dragPixels(tremor, pxPerDeg)).toBeCloseTo(1, 6);
    expect(isComplete(tremor, pxPerDeg)).toBe(false);
  });

  it("are committed once the drag passes the threshold", () => {
    const dragged: InProgress = {
      kind: "extent",
      centre: at(10, 20),
      rim: at(10 + MIN_DRAG_PX / pxPerDeg, 20),
    };
    expect(dragPixels(dragged, pxPerDeg)).toBeCloseTo(MIN_DRAG_PX, 6);
    expect(isComplete(dragged, pxPerDeg)).toBe(true);
  });

  /**
   * The threshold is a distance on *screen*, so the same hand movement means
   * the same thing at any zoom. A drag that counts when zoomed in must not
   * count when the same degrees are a quarter the pixels.
   */
  it("measure the threshold on screen, not on the ground", () => {
    const drag: InProgress = { kind: "extent", centre: at(0, 0), rim: at(0.5, 0) };
    expect(isComplete(drag, 40)).toBe(true);
    expect(isComplete(drag, 4)).toBe(false);
  });

  /** A drag across the dateline is a drag, not a lap of the world. */
  it("measure a drag across the dateline the short way", () => {
    const across: Extract<InProgress, { kind: "extent" }> = {
      kind: "extent",
      centre: at(179, 0),
      rim: at(-179, 0),
    };
    expect(dragPixels(across, pxPerDeg)).toBeCloseTo(2 * pxPerDeg, 6);
  });
});

describe("press", () => {
  /** A stamp is the whole gesture: there is nothing to hold on to. */
  it("commits a stamp on the press", () => {
    expect(press("point", null, at(3, 4), false)).toEqual({
      act: "commit",
      gesture: { kind: "point", at: [3, 4] },
    });
  });

  it("begins a stroke at the press", () => {
    expect(press("stroke", null, at(3, 4), false)).toEqual({
      act: "draw",
      drawing: { kind: "stroke", points: [[3, 4]] },
      shapeHandles: false,
    });
  });

  it("adds a vertex to a ring already in progress", () => {
    const current: InProgress = { kind: "ring", points: [at(0, 0), at(1, 0)] };
    const result = press("ring", current, at(1, 1), false);
    expect(result).toMatchObject({
      act: "draw",
      drawing: { kind: "ring", points: [[0, 0], [1, 0], [1, 1]] },
    });
  });

  /**
   * Clicking the first vertex closes the ring, which is how a polygon tool is
   * expected to end — but only once there is a polygon to close. Before three
   * vertices the click is an ordinary one, or a two-point ring would vanish.
   */
  it("closes a ring on its first vertex, but only once it has three", () => {
    const two: InProgress = { kind: "ring", points: [at(0, 0), at(1, 0)] };
    expect(press("ring", two, at(0, 0), true).act).toBe("draw");

    const three: InProgress = { kind: "ring", points: [at(0, 0), at(1, 0), at(1, 1)] };
    expect(press("ring", three, at(0, 0), true)).toEqual({ act: "close" });
  });

  /** A press away from the first vertex extends the ring however many it has. */
  it("does not close a ring on a press that landed elsewhere", () => {
    const three: InProgress = { kind: "ring", points: [at(0, 0), at(1, 0), at(1, 1)] };
    expect(press("ring", three, at(2, 2), false).act).toBe("draw");
  });

  /**
   * A path node is placed on the press and shaped while the button is held,
   * which is why the press asks for the handles rather than the release.
   */
  it("places a path node and asks to shape it", () => {
    const result = press("path", null, at(5, 6), false);
    expect(result).toEqual({
      act: "draw",
      drawing: { kind: "path", nodes: [{ at: [5, 6] }] },
      shapeHandles: true,
    });
  });

  it("appends to a path already in progress", () => {
    const current: InProgress = { kind: "path", nodes: [{ at: at(0, 0) }] };
    const result = press("path", current, at(2, 0), false);
    expect((result as { drawing: Extract<InProgress, { kind: "path" }> }).drawing.nodes).toHaveLength(
      2,
    );
  });

  /** A press must never mutate the gesture it was handed. */
  it("leaves the gesture it was given alone", () => {
    const current: InProgress = { kind: "ring", points: [at(0, 0)] };
    press("ring", current, at(1, 1), false);
    expect(current.points).toHaveLength(1);
  });
});

describe("release", () => {
  /**
   * The gestures bounded by the pointer end on the release; the ones built up
   * click by click are held until they are closed or abandoned. Getting this
   * backwards would either commit a one-vertex polygon or leave a brush stroke
   * painting for ever.
   */
  it("finishes what the pointer bounds and holds what it does not", () => {
    expect(release({ kind: "stroke", points: [at(0, 0)] })).toBe("finish");
    expect(release({ kind: "extent", centre: at(0, 0), rim: at(1, 1) })).toBe("finish");
    expect(release({ kind: "ring", points: [at(0, 0)] })).toBe("hold");
    expect(release({ kind: "path", nodes: [{ at: at(0, 0) }] })).toBe("hold");
  });

  it("finishes when there is nothing in progress", () => {
    expect(release(null)).toBe("finish");
  });
});

describe("isComplete", () => {
  it("needs three vertices for a polygon and two nodes for a path", () => {
    expect(isComplete({ kind: "ring", points: [at(0, 0), at(1, 0)] }, pxPerDeg)).toBe(false);
    expect(
      isComplete({ kind: "ring", points: [at(0, 0), at(1, 0), at(1, 1)] }, pxPerDeg),
    ).toBe(true);

    expect(isComplete({ kind: "path", nodes: [{ at: at(0, 0) }] }, pxPerDeg)).toBe(false);
    expect(
      isComplete({ kind: "path", nodes: [{ at: at(0, 0) }, { at: at(1, 0) }] }, pxPerDeg),
    ).toBe(true);
  });

  /** A one-point stroke is a stamp of the brush, which is a real gesture. */
  it("accepts a stroke of one point", () => {
    expect(isComplete({ kind: "stroke", points: [at(0, 0)] }, pxPerDeg)).toBe(true);
    expect(isComplete({ kind: "stroke", points: [] }, pxPerDeg)).toBe(false);
  });
});

describe("shapeNode", () => {
  /**
   * The handles are symmetric about the node, which is what makes a smooth
   * curve through it rather than a cusp. Asymmetric handles would still draw,
   * so this is a rule only a test states.
   */
  it("pulls a node's handles out symmetrically", () => {
    const drawing: Extract<InProgress, { kind: "path" }> = {
      kind: "path",
      nodes: [{ at: at(10, 20) }],
    };
    shapeNode(drawing, at(12, 23));

    const node = drawing.nodes[0]!;
    expect(node.out_handle).toEqual([12, 23]);
    expect(node.in_handle).toEqual([8, 17]);

    // The node itself is the midpoint of its two handles.
    expect((node.in_handle![0] + node.out_handle![0]) / 2).toBeCloseTo(node.at[0], 9);
    expect((node.in_handle![1] + node.out_handle![1]) / 2).toBeCloseTo(node.at[1], 9);
  });

  it("does nothing to a path with no nodes", () => {
    const empty: Extract<InProgress, { kind: "path" }> = { kind: "path", nodes: [] };
    expect(() => shapeNode(empty, at(0, 0))).not.toThrow();
  });
});

describe("gestureLatitude", () => {
  /**
   * Where the gesture *began*, in every case. Taking it from wherever the
   * pointer finished would resize a px footprint as a drag moved north, and the
   * preview would stop agreeing with what got painted.
   */
  it("is where each gesture began", () => {
    expect(gestureLatitude({ kind: "stroke", points: [at(0, 30), at(0, 60)] })).toBe(30);
    expect(gestureLatitude({ kind: "point", at: at(0, 45) })).toBe(45);
    expect(gestureLatitude({ kind: "extent", centre: at(0, 12), rim: at(9, 70) })).toBe(12);
    expect(gestureLatitude({ kind: "ring", points: [at(0, -5), at(1, 40)] })).toBe(-5);
    expect(gestureLatitude({ kind: "path", nodes: [{ at: at(0, 8) }, { at: at(1, 50) }] })).toBe(8);
  });

  it("answers zero for a gesture with nothing in it", () => {
    expect(gestureLatitude({ kind: "stroke", points: [] })).toBe(0);
    expect(gestureLatitude({ kind: "path", nodes: [] })).toBe(0);
  });
});

describe("hoverGesture", () => {
  const schema = (gesture: string): ToolSchema =>
    ({
      tool: "brush",
      label: "",
      shortcut: "b",
      hover: true,
      preview: "field",
      gesture: { kind: "always", gesture },
      sizing: null,
      options: [],
    }) as ToolSchema;
  const state: ToolState = { values: {}, unit: "km" };

  /** A stamp's hover is the stamp; anything else's is the one-point stroke a
   * click would paint. */
  it("previews what a click would produce", () => {
    expect(hoverGesture(schema("point"), state, { lon: 3, lat: 4 })).toEqual({
      kind: "point",
      at: [3, 4],
    });
    expect(hoverGesture(schema("stroke"), state, { lon: 3, lat: 4 })).toEqual({
      kind: "stroke",
      points: [[3, 4]],
    });
  });
});
