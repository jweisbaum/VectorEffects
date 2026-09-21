/**
 * Image layers: the warp-mesh path only (spec.md 4.9).
 *
 * The texture cache itself needs a real `WebGL2RenderingContext` and is
 * exercised through the running application; what is checkable here without
 * one is the pure data on its way to the GPU — the pass-through the whole
 * "works in every projection" claim rests on, and the locality assumption
 * that justifies drawing a warp through a mesh at all.
 */
import { describe, expect, it } from "vitest";

import type { ImageLayerView } from "../generated/ImageLayerView";
import { currentHint, reportError, setHint } from "../hint";
import { WarpSpanWarnings, warpMeshFor, warpSpansTooMuch } from "./images";

/** A minimal, fully-shaped view; each test overrides what it cares about. */
function baseView(overrides: Partial<ImageLayerView>): ImageLayerView {
  return {
    layer: 0,
    path: "/chart.png",
    loaded: true,
    width: 1000,
    height: 800,
    placement: [0.001, 0, -70, 0, -0.001, 42],
    opacity: 1,
    control_points: [],
    warped: false,
    warp_mesh: [],
    warp_cells: 0,
    corners: [
      [-70, 42],
      [-69, 42],
      [-69, 41],
      [-70, 41],
    ],
    corner_residual_deg: [0, 0, 0, 0],
    georeferenced: false,
    ...overrides,
  };
}

/** A harbour-scale warp: a few kilometres across. */
function warpedImageView(): ImageLayerView {
  return baseView({
    warped: true,
    warp_cells: 2,
    // A tiny 3x3 grid — enough points to be a real mesh, small enough to
    // read by eye. Not a round number anywhere, so a transposed or
    // resampled copy would not pass by coincidence.
    warp_mesh: [
      -70.01, 42.02, -70.005, 42.021, -70.0, 42.02, -70.011, 42.01, -70.006, 42.011, -70.001,
      42.009, -70.012, 42.0, -70.007, 42.001, -70.002, 42.0,
    ],
  });
}

describe("a warped image's mesh (spec.md 4.9)", () => {
  /**
   * The warped path hands the vertex shader lon/lat directly, and Rust is
   * what evaluates the warp — projecting, normalising or reordering it in
   * TypeScript on the way would silently undo the reason this works in
   * every projection, since the shader would then be projecting an already
   * distorted mesh. The conversion here must therefore be a straight
   * pass-through: the same numbers, the same order, the same count.
   */
  it("passes the warp mesh through to the vertex buffer untouched", () => {
    const view = warpedImageView();
    const mesh = warpMeshFor(view);
    expect(mesh).not.toBeNull();
    // Compared against the same, and only, change `Float32Array` itself
    // makes — narrowing precision — never against the original numbers
    // directly: an f64 does not survive that narrowing exactly, and a test
    // that expected it to would be asserting against the wrong reference.
    expect(Array.from(mesh!)).toEqual(Array.from(Float32Array.from(view.warp_mesh)));
    expect(mesh).toHaveLength(view.warp_mesh.length);
  });

  it("has no mesh for a plain affine image", () => {
    expect(warpMeshFor(baseView({ warped: false }))).toBeNull();
  });

  /** `warped` false is what says "use the affine", regardless of leftover data. */
  it("has no mesh when warped is false even if warp_mesh is not empty", () => {
    expect(warpMeshFor(baseView({ warped: false, warp_mesh: [1, 2, 3, 4] }))).toBeNull();
  });

  it("has no mesh when warped is true but the mesh is empty", () => {
    expect(warpMeshFor(baseView({ warped: true, warp_mesh: [] }))).toBeNull();
  });
});

describe("the locality assumption behind the mesh/per-pixel split", () => {
  /**
   * The spec justifies drawing a warp through a mesh, rather than inverting
   * it per pixel the way an unwarped image's globe path does, on the grounds
   * that a rubber-sheeted image is *local*: a harbour, an approach, a
   * scanned sheet. A warp whose corners are spread across a good fraction of
   * the earth is outside what that reasoning covers, and the renderer says
   * so — through `reportError`, in the caller — rather than quietly drawing
   * a coarse mesh across a hemisphere.
   */
  it("says when a warped image is too large for the mesh path", () => {
    // Corners a quarter of the way round the equator and back: tens of
    // thousands of kilometres, nothing a control-point warp is meant for.
    const hugeMeshSpanningHalfTheEarth = [-90, 0, 90, 0, 90, -40, -90, -40];
    // A few hundred metres across a harbour mouth.
    const harbourSizedMesh = [4.1, 52.01, 4.102, 52.01, 4.102, 52.008, 4.1, 52.008];
    expect(warpSpansTooMuch(hugeMeshSpanningHalfTheEarth)).toBe(true);
    expect(warpSpansTooMuch(harbourSizedMesh)).toBe(false);
  });

  it("says nothing about too few points to have a footprint", () => {
    expect(warpSpansTooMuch([])).toBe(false);
    expect(warpSpansTooMuch([1, 2])).toBe(false);
  });
});

describe("WarpSpanWarnings (spec.md 4.9, review finding 2)", () => {
  const hugeMesh = [-90, 0, 90, 0, 90, -40, -90, -40];
  const harbourMesh = [4.1, 52.01, 4.102, 52.01, 4.102, 52.008, 4.1, 52.008];

  /**
   * The regression this closes: `draws()` used to call `reportError` on
   * every frame a too-large warp existed. `hint.ts`'s `publish` dedupes an
   * unchanged snapshot, so calling `update` with the *same* mesh identity
   * twice — standing in for two frames of a still scene — must leave the
   * error exactly as it was, not merely equal in value: a hint set in
   * between must survive, which it would not if `update` re-asserted the
   * error unconditionally.
   */
  it("reports once and does not re-assert on an unchanged mesh", () => {
    setHint(null);
    reportError(null);
    const warnings = new WarpSpanWarnings();
    warnings.update(1, "/chart.png", hugeMesh);
    expect(currentHint().error).toContain("quarter of the earth");

    // A hint set after the warning must not be clobbered by a later frame
    // whose mesh identity has not changed — the bug this closes would call
    // reportError again here and stamp the error straight back over it.
    setHint("Click the map to place it.");
    warnings.update(1, "/chart.png", hugeMesh);
    expect(currentHint().hint).toBe("Click the map to place it.");
    expect(currentHint().error).toBeNull();
    setHint(null);
  });

  it("clears the warning once the warp no longer spans too much", () => {
    reportError(null);
    const warnings = new WarpSpanWarnings();
    warnings.update(1, "/chart.png", hugeMesh);
    expect(currentHint().error).not.toBeNull();
    warnings.update(1, "/chart.png", harbourMesh);
    expect(currentHint().error).toBeNull();
  });

  it("clears the warning when the layer stops being warped or is removed", () => {
    reportError(null);
    const warnings = new WarpSpanWarnings();
    warnings.update(1, "/chart.png", hugeMesh);
    expect(currentHint().error).not.toBeNull();
    warnings.forget(1);
    expect(currentHint().error).toBeNull();
  });

  /** Never steps on an unrelated error that has since taken the line. */
  it("never clears an error it did not itself set", () => {
    reportError(null);
    const warnings = new WarpSpanWarnings();
    warnings.update(1, "/chart.png", hugeMesh);
    reportError("the layer is locked", "bad-option");
    warnings.forget(1);
    expect(currentHint().error).toBe("the layer is locked");
    reportError(null);
  });

  it("keeps two layers' warnings independent", () => {
    reportError(null);
    const warnings = new WarpSpanWarnings();
    warnings.update(1, "/a.png", hugeMesh);
    warnings.update(2, "/b.png", hugeMesh);
    expect(currentHint().error).toContain("/b.png");
    // Clearing the first must not disturb the second's still-active report.
    warnings.forget(1);
    expect(currentHint().error).toContain("/b.png");
    warnings.forget(2);
    expect(currentHint().error).toBeNull();
  });
});
