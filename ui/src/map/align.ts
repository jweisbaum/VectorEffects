/**
 * The image alignment mode (Task 7, spec.md §4.9 §2, §5): the interaction
 * behind the "Align…" button. The user clicks a feature in the picture, then
 * the place it belongs on the map, as many times as they like, then presses
 * Enter — which writes the whole set through `setImageControlPoints` as one
 * undo entry (spec.md's "one clipboard"-style full replacement, the same
 * shape `Command::SetAnnotations` takes for a measurement).
 *
 * An external store, **not** React state (see `createReadoutStore` in
 * `MapView.tsx`): a click is rare compared to a pointer move, but the pending
 * half of a pair and the growing pair list still change outside any render
 * React would schedule on its own, and putting them in component state would
 * be the same mistake the readout store exists to avoid — a place this
 * codebase has already paid for once.
 */

import type { GeoPoint } from "./camera";

/** Which half of a pair the next click completes. */
export type AlignKind = "picture" | "map";

/** One finished control point, in the shape `setImageControlPoints` wants. */
export interface AlignPair {
  u: number;
  v: number;
  lon: number;
  lat: number;
}

/** What the store holds, and what a subscriber reads back. */
export interface AlignSnapshot {
  /** The layer being aligned, or null while the mode is off. */
  layer: number | null;
  /** Which half of the next pair a click completes. */
  expecting: AlignKind;
  /** Every pair placed so far this session. */
  pairs: AlignPair[];
  /**
   * The picture half of a pair not yet completed by a map click, in the
   * image's own pixel space — kept so the overlay can draw a line from it to
   * the pointer while the map half is still pending. Not a pair: a picture
   * click with no map click yet is discarded, never sent.
   */
  pending: { u: number; v: number } | null;
}

const INITIAL: AlignSnapshot = { layer: null, expecting: "picture", pairs: [], pending: null };

/**
 * A fresh alignment session's state machine, driven by `begin`, `pick`,
 * `undoLast` and `cancel`.
 *
 * Every session starts with no pairs, whatever the image's own
 * `control_points` already hold: this is a single alignment pass, not an
 * editor of a previous one, and `set_image_control_points` replaces the
 * whole list on `Enter` regardless.
 */
export function createAlignStore() {
  let snapshot: AlignSnapshot = INITIAL;
  const listeners = new Set<() => void>();

  function publish(next: AlignSnapshot) {
    snapshot = next;
    for (const listener of listeners) listener();
  }

  return {
    get: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    /** Arms the mode for `layer`, starting on the picture with nothing placed. */
    begin(layer: number) {
      publish({ layer, expecting: "picture", pairs: [], pending: null });
    },
    /**
     * Takes one click's worth of position, for whichever half of the pair
     * `expecting` says it is. A `kind` that does not match `expecting` is
     * ignored rather than reordering the pair — the caller alternates by
     * reading `expecting` itself, so this only guards a caller that does not.
     */
    pick(kind: AlignKind, at: GeoPoint) {
      if (snapshot.layer === null || kind !== snapshot.expecting) return;
      if (kind === "picture") {
        publish({ ...snapshot, expecting: "map", pending: { u: at.lon, v: at.lat } });
        return;
      }
      const pending = snapshot.pending;
      if (pending === null) return;
      const pair: AlignPair = { u: pending.u, v: pending.v, lon: at.lon, lat: at.lat };
      publish({ ...snapshot, expecting: "picture", pairs: [...snapshot.pairs, pair], pending: null });
    },
    /**
     * Backspace: drops the picture click waiting for its map half, if there
     * is one, otherwise the last completed pair. Two presses undo one
     * half-placed click and then one whole pair, matching what a user sees
     * on screen at each step rather than always popping the pair list.
     */
    undoLast() {
      if (snapshot.pending !== null) {
        publish({ ...snapshot, expecting: "picture", pending: null });
        return;
      }
      publish({ ...snapshot, pairs: snapshot.pairs.slice(0, -1) });
    },
    /** Escape: leaves the mode with nothing written. */
    cancel() {
      publish(INITIAL);
    },
    pairs: () => snapshot.pairs,
  };
}

export type AlignStore = ReturnType<typeof createAlignStore>;

/** The minimum an `ImageLayerView` needs for `imagePixelAt` and `pictureToMap`. */
export interface AlignableView {
  width: number;
  height: number;
  /** `[a, b, c, d, e, f]`: `lon = a*u + b*v + c`, `lat = d*u + e*v + f`. */
  placement: number[];
  warped?: boolean;
  /** Flat, row-major from the top-left: `[lon0, lat0, lon1, lat1, ...]`. */
  warp_mesh?: number[];
  /** The mesh is `(warp_cells + 1)` square. */
  warp_cells?: number;
}

/**
 * A map position, carried back to the image's own pixel space through the
 * view's *current* warp.
 *
 * **Unwarped**: the 2×2 affine in `placement` inverted directly — the same
 * six numbers `placeVectors` (`images.ts`) reads to draw the image, run
 * backwards.
 *
 * **Warped**: a thin-plate spline has no closed-form inverse (the same fact
 * that forced the mesh render path), so this searches `warp_mesh` — the
 * forward map already evaluated on a grid — for the cell containing `at`,
 * then inverts *that* cell's bilinear patch by Newton's method. Bilinear
 * inversion is exact once the cell is found; the only approximation is the
 * mesh's own resolution standing in for the true spline, the same trade the
 * render path already makes. At 64 cells over an image that is far finer
 * than anyone can click by eye.
 */
export function imagePixelAt(view: AlignableView, at: GeoPoint): [number, number] {
  const mesh = view.warped ? view.warp_mesh : undefined;
  if (mesh && mesh.length > 0 && view.warp_cells) {
    // Neither `place` nor `mesh` normalises longitude on the Rust side, so an
    // image across the antimeridian keeps its right edge past 180° rather
    // than folding (`ve_core::warp` says so explicitly, and tests it). A
    // click's own lon/lat comes back from `unproject` normalised to
    // [-180, 180), so it is unwrapped to the mesh's own copy of the world
    // before the search — matching what `project` already does for the
    // outline's hit test — or a click meant to be 182° would be searched for
    // at -178° and never find its cell.
    const lon = unwrapLon(at.lon, mesh[0]!);
    return imagePixelFromMesh(mesh, view.warp_cells, view.width, view.height, { lon, lat: at.lat });
  }
  const lon = unwrapLon(at.lon, view.placement[2]!);
  return imagePixelFromAffine(view.placement, { lon, lat: at.lat });
}

/** `lon`, shifted by whole turns to the copy of the world nearest `reference`. */
function unwrapLon(lon: number, reference: number): number {
  return lon + Math.round((reference - lon) / 360) * 360;
}

/** The forward map: an image pixel to where it currently sits on the map. */
export function pictureToMap(view: AlignableView, u: number, v: number): GeoPoint {
  const mesh = view.warped ? view.warp_mesh : undefined;
  if (mesh && mesh.length > 0 && view.warp_cells) {
    return pictureFromMesh(mesh, view.warp_cells, view.width, view.height, u, v);
  }
  const [a, b, c, d, e, f] = view.placement;
  return { lon: a! * u + b! * v + c!, lat: d! * u + e! * v + f! };
}

function imagePixelFromAffine(placement: number[], at: GeoPoint): [number, number] {
  const [a, b, c, d, e, f] = placement;
  const det = a! * e! - b! * d!;
  const lonC = at.lon - c!;
  const latF = at.lat - f!;
  if (Math.abs(det) < 1e-15) return [0, 0];
  const u = (e! * lonC - b! * latF) / det;
  const v = (a! * latF - d! * lonC) / det;
  return [u, v];
}

/** A mesh vertex's lon/lat, as the plain tuple the geometry below wants. */
function meshVertex(mesh: number[], cells: number, row: number, col: number): [number, number] {
  const i = (row * (cells + 1) + col) * 2;
  return [mesh[i]!, mesh[i + 1]!];
}

/** Even-odd point-in-polygon, walking the quad's own perimeter. */
function pointInQuad(at: GeoPoint, quad: [number, number][]): boolean {
  let inside = false;
  for (let i = 0, j = quad.length - 1; i < quad.length; j = i++) {
    const a = quad[i]!;
    const b = quad[j]!;
    const crosses = a[1] > at.lat !== b[1] > at.lat;
    if (crosses && at.lon < ((b[0] - a[0]) * (at.lat - a[1])) / (b[1] - a[1]) + a[0]) {
      inside = !inside;
    }
  }
  return inside;
}

/**
 * Inverts one cell's bilinear patch by Newton's method, starting at the
 * cell's centre. A bilinear map is exact, so this converges to machine
 * precision in a handful of iterations — there is no approximation here
 * beyond the mesh standing in for the spline between its own vertices.
 */
function invertBilinearCell(
  p00: [number, number],
  p10: [number, number],
  p01: [number, number],
  p11: [number, number],
  target: GeoPoint,
): { s: number; t: number } {
  let s = 0.5;
  let t = 0.5;
  for (let iter = 0; iter < 20; iter++) {
    const x =
      (1 - s) * (1 - t) * p00[0] + s * (1 - t) * p10[0] + (1 - s) * t * p01[0] + s * t * p11[0];
    const y =
      (1 - s) * (1 - t) * p00[1] + s * (1 - t) * p10[1] + (1 - s) * t * p01[1] + s * t * p11[1];
    const fx = x - target.lon;
    const fy = y - target.lat;
    const dxds = (1 - t) * (p10[0] - p00[0]) + t * (p11[0] - p01[0]);
    const dxdt = (1 - s) * (p01[0] - p00[0]) + s * (p11[0] - p10[0]);
    const dyds = (1 - t) * (p10[1] - p00[1]) + t * (p11[1] - p01[1]);
    const dydt = (1 - s) * (p01[1] - p00[1]) + s * (p11[1] - p10[1]);
    const det = dxds * dydt - dxdt * dyds;
    if (Math.abs(det) < 1e-18) break;
    const ds = (-fx * dydt + fy * dxdt) / det;
    const dt = (-fy * dxds + fx * dyds) / det;
    s += ds;
    t += dt;
    if (Math.abs(ds) < 1e-12 && Math.abs(dt) < 1e-12) break;
  }
  return { s, t };
}

function imagePixelFromMesh(
  mesh: number[],
  cells: number,
  width: number,
  height: number,
  at: GeoPoint,
): [number, number] {
  let nearest: { row: number; col: number; dist: number } | null = null;
  for (let row = 0; row < cells; row++) {
    for (let col = 0; col < cells; col++) {
      const p00 = meshVertex(mesh, cells, row, col);
      const p10 = meshVertex(mesh, cells, row, col + 1);
      const p11 = meshVertex(mesh, cells, row + 1, col + 1);
      const p01 = meshVertex(mesh, cells, row + 1, col);
      // Walked in perimeter order — p00, p10, p11, p01 — not the bilinear
      // corner order used below, which would cross itself as a polygon.
      if (pointInQuad(at, [p00, p10, p11, p01])) {
        const { s, t } = invertBilinearCell(p00, p10, p01, p11, at);
        return cellToPixel(row, col, s, t, cells, width, height);
      }
      const cx = (p00[0] + p10[0] + p01[0] + p11[0]) / 4;
      const cy = (p00[1] + p10[1] + p01[1] + p11[1]) / 4;
      const dist = (cx - at.lon) ** 2 + (cy - at.lat) ** 2;
      if (nearest === null || dist < nearest.dist) nearest = { row, col, dist };
    }
  }
  // A click a hair outside every cell — the mesh's own edge, or a warp so
  // extreme the corner sits outside its neighbours' hull. Newton still
  // converges; it simply extrapolates the nearest patch a little.
  const { row, col } = nearest!;
  const p00 = meshVertex(mesh, cells, row, col);
  const p10 = meshVertex(mesh, cells, row, col + 1);
  const p11 = meshVertex(mesh, cells, row + 1, col + 1);
  const p01 = meshVertex(mesh, cells, row + 1, col);
  const { s, t } = invertBilinearCell(p00, p10, p01, p11, at);
  return cellToPixel(row, col, s, t, cells, width, height);
}

function cellToPixel(
  row: number,
  col: number,
  s: number,
  t: number,
  cells: number,
  width: number,
  height: number,
): [number, number] {
  const cellW = width / cells;
  const cellH = height / cells;
  return [(col + s) * cellW, (row + t) * cellH];
}

function pictureFromMesh(
  mesh: number[],
  cells: number,
  width: number,
  height: number,
  u: number,
  v: number,
): GeoPoint {
  const cellW = width / cells;
  const cellH = height / cells;
  const colF = Math.min(Math.max(u / cellW, 0), cells);
  const rowF = Math.min(Math.max(v / cellH, 0), cells);
  const col = Math.min(Math.floor(colF), cells - 1);
  const row = Math.min(Math.floor(rowF), cells - 1);
  const s = colF - col;
  const t = rowF - row;
  const p00 = meshVertex(mesh, cells, row, col);
  const p10 = meshVertex(mesh, cells, row, col + 1);
  const p01 = meshVertex(mesh, cells, row + 1, col);
  const p11 = meshVertex(mesh, cells, row + 1, col + 1);
  const lon = (1 - s) * (1 - t) * p00[0] + s * (1 - t) * p10[0] + (1 - s) * t * p01[0] + s * t * p11[0];
  const lat = (1 - s) * (1 - t) * p00[1] + s * (1 - t) * p10[1] + (1 - s) * t * p01[1] + s * t * p11[1];
  return { lon, lat };
}
