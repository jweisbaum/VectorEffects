import { describe, expect, it } from "vitest";

import { createAlignStore, imagePixelAt, pictureToMap, type AlignableView } from "./align";

describe("createAlignStore", () => {
  /**
   * The mode alternates, starting on the picture, and a half-finished pair is
   * not a pair. This is the whole state machine and it is worth testing away
   * from the map, where it can be driven directly.
   */
  it("alternates picture then map, and discards a half-placed pair", () => {
    const store = createAlignStore();
    store.begin(7);
    expect(store.get().expecting).toBe("picture");
    store.pick("picture", { lon: -70, lat: 41 });
    expect(store.get().expecting).toBe("map");
    expect(store.pairs()).toHaveLength(0);
    store.pick("map", { lon: -69, lat: 42 });
    expect(store.get().expecting).toBe("picture");
    expect(store.pairs()).toHaveLength(1);
    store.pick("picture", { lon: -70, lat: 40 });
    expect(store.pairs()).toHaveLength(1);
  });

  /** Backspace drops the last pair, and then the one before it. */
  it("undoes the last pair", () => {
    const store = createAlignStore();
    store.begin(7);
    store.pick("picture", { lon: -70, lat: 41 });
    store.pick("map", { lon: -69, lat: 42 });
    store.pick("picture", { lon: -71, lat: 41 });
    store.pick("map", { lon: -68, lat: 42 });
    expect(store.pairs()).toHaveLength(2);
    store.undoLast();
    expect(store.pairs()).toHaveLength(1);
    expect(store.get().expecting).toBe("picture");
  });

  /**
   * Backspace mid-pair drops the waiting picture click first, rather than
   * reaching past it to a pair that is already complete and on screen.
   */
  it("undoes a half-placed picture click before it undoes a whole pair", () => {
    const store = createAlignStore();
    store.begin(7);
    store.pick("picture", { lon: -70, lat: 41 });
    store.pick("map", { lon: -69, lat: 42 });
    store.pick("picture", { lon: -71, lat: 41 }); // waiting for its map half
    expect(store.get().expecting).toBe("map");
    store.undoLast();
    expect(store.get().expecting).toBe("picture");
    expect(store.pairs()).toHaveLength(1); // the first pair is untouched
  });

  /** Escape leaves the document untouched: nothing placed survives it. */
  it("cancels the whole session", () => {
    const store = createAlignStore();
    store.begin(7);
    store.pick("picture", { lon: -70, lat: 41 });
    store.pick("map", { lon: -69, lat: 42 });
    store.cancel();
    expect(store.get().layer).toBeNull();
    expect(store.pairs()).toHaveLength(0);
  });

  /** Starting over drops whatever a previous session on another layer left. */
  it("begin resets pairs even mid-session", () => {
    const store = createAlignStore();
    store.begin(7);
    store.pick("picture", { lon: -70, lat: 41 });
    store.pick("map", { lon: -69, lat: 42 });
    store.begin(9);
    expect(store.get().layer).toBe(9);
    expect(store.get().expecting).toBe("picture");
    expect(store.pairs()).toHaveLength(0);
  });

  it("notifies subscribers on every change", () => {
    const store = createAlignStore();
    let calls = 0;
    const unsubscribe = store.subscribe(() => {
      calls += 1;
    });
    store.begin(7);
    store.pick("picture", { lon: -70, lat: 41 });
    store.pick("map", { lon: -69, lat: 42 });
    expect(calls).toBe(3);
    unsubscribe();
    store.undoLast();
    expect(calls).toBe(3);
  });
});

describe("imagePixelAt", () => {
  /**
   * A picture click is carried back to an image pixel through the warp in
   * force, so pairs accumulate against a stable image space however many have
   * already been placed. Checked against the inverse of a known placement.
   */
  it("turns a map position into the image pixel under it", () => {
    const view = { width: 800, height: 600, placement: [1 / 800, 0, -71, 0, -1 / 600, 42] };
    expect(imagePixelAt(view, { lon: -70.5, lat: 41.5 })).toEqual([400, 300]);
  });

  /**
   * `Placement::place` does not normalise longitude, so an image across the
   * antimeridian keeps its right edge past 180°. A click there comes back
   * from `unproject` normalised into [-180, 180) — the invariant every other
   * caller relies on — so `imagePixelAt` has to unwrap it to the placement's
   * own copy of the world before inverting, or a genuine click at 182° would
   * be solved as -178° and land nowhere near the pixel it was over.
   */
  it("unwraps a click across the antimeridian for an unwarped placement", () => {
    // The image's left edge is at 178°, its right edge (400px across) at 182°.
    const view = { width: 400, height: 200, placement: [0.01, 0, 178, 0, -0.01, 1] };
    // A click at the image's right edge, which `unproject` would report as
    // -178, not 182.
    expect(imagePixelAt(view, { lon: -178, lat: 0 })).toEqual([400, 100]);
  });

  it("round-trips through a rotated affine placement", () => {
    // A placement with rotation and non-uniform scale: not axis-aligned, so
    // an inverse that only handled the diagonal case would still pass the
    // brief's own test above.
    const view = { width: 200, height: 100, placement: [0.02, 0.01, -71, -0.005, 0.03, 42] };
    const pixels: [number, number][] = [
      [0, 0],
      [200, 0],
      [200, 100],
      [0, 100],
      [73, 41],
    ];
    for (const [u, v] of pixels) {
      const [a, b, c, d, e, f] = view.placement as [number, number, number, number, number, number];
      const lon = a * u + b * v + c;
      const lat = d * u + e * v + f;
      const [gotU, gotV] = imagePixelAt(view, { lon, lat });
      expect(gotU).toBeCloseTo(u, 6);
      expect(gotV).toBeCloseTo(v, 6);
    }
  });

  /**
   * The warped path has no closed-form inverse to check against (that is
   * the whole reason it exists), so it is checked the way the amendment
   * asks: build a mesh the way `Warp::mesh` does — evaluated forward on a
   * grid — from a target function no affine could reach, then confirm
   * `imagePixelAt` recovers the pixel that produced each mesh vertex, and a
   * point in between a vertex round-trips through `pictureToMap` too.
   */
  it("inverts a warp through its mesh, bilinearly, for a target no affine reaches", () => {
    const width = 800;
    const height = 600;
    const cells = 8;
    // A genuine bend: a sine ripple, the same shape the Rust warp suite
    // uses to prove the spline is not secretly an affine.
    const forward = (u: number, v: number) => ({
      lon: -71 + u / 800 + 0.05 * Math.sin(v / 100),
      lat: 42 - v / 600 + 0.05 * Math.cos(u / 100),
    });
    const mesh: number[] = [];
    for (let row = 0; row <= cells; row++) {
      const v = (row / cells) * height;
      for (let col = 0; col <= cells; col++) {
        const u = (col / cells) * width;
        const { lon, lat } = forward(u, v);
        mesh.push(lon, lat);
      }
    }
    const view: AlignableView = {
      width,
      height,
      placement: [1 / width, 0, -71, 0, -1 / height, 42],
      warped: true,
      warp_mesh: mesh,
      warp_cells: cells,
    };

    // Every mesh vertex must invert back to the pixel it was sampled at.
    for (let row = 0; row <= cells; row++) {
      for (let col = 0; col <= cells; col++) {
        const u = (col / cells) * width;
        const v = (row / cells) * height;
        const { lon, lat } = forward(u, v);
        const [gotU, gotV] = imagePixelAt(view, { lon, lat });
        expect(gotU).toBeCloseTo(u, 3);
        expect(gotV).toBeCloseTo(v, 3);
      }
    }

    // A point strictly inside one cell, checked against the analytic warp
    // rather than against this module's own forward helper.
    const u = 130;
    const v = 47;
    const target = forward(u, v);
    const [gotU, gotV] = imagePixelAt(view, target);
    // One cell spans width/cells = 100px by height/cells = 75px; the mesh's
    // own piecewise-bilinear approximation of the sine, not the exact
    // curve, bounds the achievable accuracy here — a fraction of a cell,
    // nowhere near a whole one, and far finer than a click by eye.
    expect(Math.abs(gotU - u)).toBeLessThan(2);
    expect(Math.abs(gotV - v)).toBeLessThan(2);

    // The forward direction round-trips the same way: a mesh vertex maps
    // back to exactly the pixel it was built from.
    const back = pictureToMap(view, 300, 225);
    const expected = forward(300, 225);
    expect(back.lon).toBeCloseTo(expected.lon, 6);
    expect(back.lat).toBeCloseTo(expected.lat, 6);
  });

  /**
   * A spline's mesh is not normalised either (`ve_core::warp::Warp::place`
   * "is not normalised, for the same reason `Placement::place` does not").
   * Built here spanning the seam the way the Rust suite's own
   * `a_spline_across_the_antimeridian_is_not_folded` does, with a click
   * `unproject` would report on the wrapped side.
   */
  it("unwraps a click across the antimeridian for a warped mesh", () => {
    const width = 800;
    const height = 600;
    const cells = 4;
    // Straddles the seam: 178° at the left edge, 182° at the right.
    const forward = (u: number, v: number) => ({
      lon: 178 + (u / width) * 4,
      lat: 1 - (v / height) * 2,
    });
    const mesh: number[] = [];
    for (let row = 0; row <= cells; row++) {
      for (let col = 0; col <= cells; col++) {
        const { lon, lat } = forward((col / cells) * width, (row / cells) * height);
        mesh.push(lon, lat);
      }
    }
    const view: AlignableView = {
      width,
      height,
      placement: [4 / width, 0, 178, -2 / height, 0, 1],
      warped: true,
      warp_mesh: mesh,
      warp_cells: cells,
    };
    // The right edge, u=800, is lon 182 — which `unproject` would hand back
    // as -178.
    const [gotU, gotV] = imagePixelAt(view, { lon: -178, lat: 0 });
    expect(gotU).toBeCloseTo(width, 1);
    expect(gotV).toBeCloseTo(height / 2, 1);
  });
});
