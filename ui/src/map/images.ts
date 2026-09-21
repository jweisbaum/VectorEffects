/**
 * Textures for image layers (spec.md 4.9, M18).
 *
 * The picture comes through the same `ve-tile://` scheme the field tiles use,
 * as a PNG the webview decodes for free: a chart scan is megabytes, which has
 * no business crossing the IPC channel as JSON.
 *
 * The address carries the document revision *and* the largest texture this
 * GPU will take, so it is immutable — re-importing an image, or opening
 * another project, makes the old address unreachable rather than stale, and
 * the downsampling is done once in Rust rather than in the webview.
 */

import type { ImageLayerView } from "../generated/ImageLayerView";
import { EARTH_RADIUS_M, distanceM } from "./geo";
import { currentHint, reportError } from "../hint";
import type { ImageDraw } from "./renderer";

/** How a fetch is doing. */
type Status = "pending" | "ready" | "failed";

interface Entry {
  texture: WebGLTexture | null;
  status: Status;
}

/**
 * A quarter of the earth's circumference, in metres: `distanceM`'s scale.
 * The threshold `warpSpansTooMuch` reports past (spec.md 4.9).
 */
const QUARTER_EARTH_M = (Math.PI * EARTH_RADIUS_M) / 2;

/**
 * Whether a warped image's footprint reaches past what the mesh/per-pixel
 * split is justified by: a rubber-sheeted image is *local* by its nature —
 * a harbour, an approach, a scanned sheet — which is why it is affordable to
 * follow with a mesh rather than inverting the warp per pixel. A warp whose
 * corners are more than a quarter of the earth apart is outside that
 * reasoning. It is not refused — the mesh still draws it — but it is
 * reported, because a mesh coarse enough for a chart is not fine enough for
 * a hemisphere and nothing else would say so.
 */
export function warpSpansTooMuch(mesh: ArrayLike<number>): boolean {
  if (mesh.length < 4) return false;
  let minLon = Infinity;
  let maxLon = -Infinity;
  let minLat = Infinity;
  let maxLat = -Infinity;
  for (let i = 0; i + 1 < mesh.length; i += 2) {
    const lon = mesh[i]!;
    const lat = mesh[i + 1]!;
    if (lon < minLon) minLon = lon;
    if (lon > maxLon) maxLon = lon;
    if (lat < minLat) minLat = lat;
    if (lat > maxLat) maxLat = lat;
  }
  const midLon = (minLon + maxLon) / 2;
  const midLat = (minLat + maxLat) / 2;
  const eastWest = distanceM({ lon: minLon, lat: midLat }, { lon: maxLon, lat: midLat });
  const northSouth = distanceM({ lon: midLon, lat: minLat }, { lon: midLon, lat: maxLat });
  return Math.max(eastWest, northSouth) > QUARTER_EARTH_M;
}

/**
 * Tracks, per image layer, whether its warp has been reported as spanning
 * more than `warpSpansTooMuch` allows for — and clears the report exactly
 * when that stops being true, rather than repeating it.
 *
 * `ImageCache.draws()` runs inside `draw()`, once a frame. Calling
 * `reportError` from there on every frame a too-large warp exists would seem
 * harmless — `hint.ts`'s `publish` dedupes identical snapshots, so there is
 * no render storm — but `setHint` only clears the *error* when the hint
 * *text* changes, and the very next frame would put this error straight
 * back. So while such an image existed, every other hint and error in the
 * application would be masked permanently, which is exactly what "no panel
 * renders an error line of its own; `reportError` and `setHint` are the
 * whole API" exists to prevent. `update` is therefore called only when a
 * layer's mesh identity actually changes (a real re-fit), never per frame.
 */
export class WarpSpanWarnings {
  private readonly active = new Map<number, string>();
  /**
   * The mesh identity `update` last saw per layer, by reference. This is
   * what makes `update` idempotent on its own — safe to call redundantly —
   * rather than depending entirely on a caller that only invokes it on a
   * real re-fit: a still scene calling it every frame with the very same
   * array must not re-assert an error a hint has since taken the place of.
   */
  private readonly lastMesh = new Map<number, ArrayLike<number>>();

  /** Reports or clears a layer's warning, from its freshly rebuilt mesh. */
  update(layer: number, path: string, mesh: ArrayLike<number>): void {
    if (this.lastMesh.get(layer) === mesh) return;
    this.lastMesh.set(layer, mesh);
    if (!warpSpansTooMuch(mesh)) {
      this.clear(layer);
      return;
    }
    const message =
      `“${path}”'s warp spans more than a quarter of the earth; the mesh path this draws it ` +
      "through is built for a local chart and may look coarse that far out.";
    this.active.set(layer, message);
    reportError(message);
  }

  /** Called when a layer stops being warped, or disappears entirely. */
  forget(layer: number): void {
    this.lastMesh.delete(layer);
    this.clear(layer);
  }

  /** Called when the whole cache is torn down — a project close or reopen. */
  forgetAll(): void {
    for (const layer of [...this.active.keys()]) this.forget(layer);
  }

  private clear(layer: number): void {
    const message = this.active.get(layer);
    if (message === undefined) return;
    this.active.delete(layer);
    // Only clear an error this instance itself set — a different error (or a
    // hint) that has since taken the line's place must not be stepped on.
    if (currentHint().error === message) reportError(null);
  }
}

/**
 * The warp mesh a view carries, as a `Float32Array` — or null off a plain
 * affine image.
 *
 * **Passed through untouched.** `view.warp_mesh` is already lon/lat,
 * evaluated in Rust; this only changes its container, never its values,
 * their order or their count. That is the whole reason a warped image draws
 * correctly in every projection (spec.md 4.9): projection happens in the
 * shader, strictly after the warp, so nothing here may project, normalise or
 * reorder it first. `ImageCache` wraps this with an identity cache so the
 * same array comes back while the mesh has not changed; this function itself
 * stays a straight conversion, which is what makes the conversion checkable
 * on its own.
 */
export function warpMeshFor(view: ImageLayerView): Float32Array | null {
  if (!view.warped || view.warp_mesh.length === 0) return null;
  return Float32Array.from(view.warp_mesh);
}

/**
 * The affine, folded into two vec3s over the *whole* image.
 *
 * The stored placement is per pixel and the mesh runs 0 to 1, so the pixel
 * counts multiply in here — which also means the shader never needs to know
 * how big the image is.
 */
export function placeVectors(
  view: ImageLayerView,
): { placeLon: [number, number, number]; placeLat: [number, number, number] } {
  const [a, b, c, d, e, f] = view.placement;
  const w = Math.max(1, view.width);
  const h = Math.max(1, view.height);
  return {
    placeLon: [a! * w, b! * h, c!],
    placeLat: [d! * w, e! * h, f!],
  };
}

/** Fetches and holds one texture per image layer. */
export class ImageCache {
  private readonly gl: WebGL2RenderingContext;
  private readonly baseUrl: string;
  /** The largest texture this GPU will take, which the address carries. */
  private readonly maxEdge: number;
  private readonly entries = new Map<string, Entry>();
  /**
   * A warped image's mesh, kept by layer and reused while `view.warp_mesh`
   * is the same array the backend last sent — so a still scene uploads
   * nothing new to the renderer, frame after frame (spec.md 4.9).
   */
  private readonly warpMeshes = new Map<number, { source: number[]; array: Float32Array }>();
  /** Reports a layer's warp spanning too much once per re-fit, not per frame. */
  private readonly warnings = new WarpSpanWarnings();
  /** Called when a fetch completes, so the caller can redraw. */
  onChange: (() => void) | null = null;
  /** Called when a fetch fails, so the failure reaches the application log. */
  onError: ((message: string) => void) | null = null;

  constructor(gl: WebGL2RenderingContext, baseUrl: string) {
    this.gl = gl;
    this.baseUrl = baseUrl;
    // Capped at 8192 to match what the backend will serve: asking for more
    // would be a request it clamps anyway, under an address that claims
    // otherwise.
    this.maxEdge = Math.min(gl.getParameter(gl.MAX_TEXTURE_SIZE) as number, 8192);
  }

  /**
   * What to draw for these image layers, fetching what is not yet here.
   *
   * Returns an entry per layer whether or not its texture has arrived: the
   * renderer skips the ones that have not, and redraws when `onChange` fires,
   * so a slow decode never blanks anything that is already on screen.
   *
   * `token` is the **opening's**, not the document's revision (M37). A
   * picture's pixels depend on its file and on nothing an edit does, so
   * addressing them by the revision made every edit a new address: dragging
   * an image re-decoded the whole chart on every pointer report and drew
   * nothing at all until the pointer was released. Where the picture goes
   * comes from the placement in `views`, which is free to change every frame.
   */
  /**
   * The images to draw, in the order given.
   *
   * `over` names the layers that sit above every visible field layer (M49):
   * those are drawn on top of the field rather than under it, because the
   * field is nearly opaque and an image the user has moved to the top of the
   * stack is one they have asked to see.
   */
  draws(
    token: number,
    views: readonly (ImageLayerView & { speedRange?: [number, number] | undefined })[],
    over: ReadonlySet<number> = new Set(),
  ): ImageDraw[] {
    const wanted = new Set<string>();
    const out: ImageDraw[] = [];
    for (const view of views) {
      if (!view.loaded || view.width === 0 || view.height === 0) continue;
      const key = `image/${token}/${view.layer}/${this.maxEdge}`;
      wanted.add(key);
      const warpMesh = this.cachedWarpMesh(view);
      out.push({
        layer: view.layer,
        texture: this.texture(key),
        opacity: view.opacity,
        speedRange: view.speedRange,
        over: over.has(view.layer),
        warpMesh,
        warpCells: view.warp_cells,
        ...placeVectors(view),
      });
    }
    // An image that is gone — deleted, or at an older revision — takes its
    // texture with it. There is no LRU here because there is no pressure: a
    // project has a handful of image layers, not a pyramid of tiles.
    for (const key of [...this.entries.keys()]) {
      if (!wanted.has(key)) this.release(key);
    }
    const stillWarped = new Set(views.filter((v) => v.warped).map((v) => v.layer));
    for (const layer of [...this.warpMeshes.keys()]) {
      if (!stillWarped.has(layer)) {
        this.warpMeshes.delete(layer);
        this.warnings.forget(layer);
      }
    }
    return out;
  }

  /**
   * The warp mesh as a `Float32Array`, or null off a plain affine image.
   *
   * Cached by the identity of `view.warp_mesh` — the plain array the backend
   * sent — not its contents: a still scene hands back the same array object
   * every time, so this hands back the same `Float32Array` every time too,
   * which is what lets the renderer skip re-uploading it (spec.md 4.9). The
   * conversion itself is `warpMeshFor`, above, kept free of this cache so it
   * stays checkable as a plain pass-through.
   */
  private cachedWarpMesh(view: ImageLayerView): Float32Array | null {
    if (!view.warped || view.warp_mesh.length === 0) {
      this.warpMeshes.delete(view.layer);
      this.warnings.forget(view.layer);
      return null;
    }
    const cached = this.warpMeshes.get(view.layer);
    if (cached && cached.source === view.warp_mesh) return cached.array;
    const array = warpMeshFor(view)!;
    this.warpMeshes.set(view.layer, { source: view.warp_mesh, array });
    // The mesh identity just changed — a real re-fit, not a redraw — which is
    // the one moment to report or clear the too-large warning (see
    // `WarpSpanWarnings`).
    this.warnings.update(view.layer, view.path, array);
    return array;
  }

  private texture(key: string): WebGLTexture | null {
    const existing = this.entries.get(key);
    if (existing) return existing.texture;
    const entry: Entry = { texture: null, status: "pending" };
    this.entries.set(key, entry);
    void this.fetch(key, entry);
    return null;
  }

  private async fetch(key: string, entry: Entry): Promise<void> {
    try {
      const response = await fetch(`${this.baseUrl}${key}`);
      if (!response.ok) throw new Error(`status ${response.status}`);
      const bitmap = await createImageBitmap(await response.blob());
      // The entry may have been released while the fetch was in flight.
      if (!this.entries.has(key)) {
        bitmap.close();
        return;
      }
      entry.texture = this.upload(bitmap);
      entry.status = entry.texture ? "ready" : "failed";
      bitmap.close();
    } catch (error) {
      entry.status = "failed";
      const message = `image ${key} failed: ${
        error instanceof Error ? error.message : String(error)
      }`;
      console.warn(message);
      this.onError?.(message);
    }
    this.onChange?.();
  }

  private upload(bitmap: ImageBitmap): WebGLTexture | null {
    const gl = this.gl;
    const texture = gl.createTexture();
    if (!texture) return null;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, bitmap);
    // LINEAR here, unlike the field tiles: a picture is a picture, and its
    // texels mean what they look like rather than carrying a 16-bit value
    // split across a byte pair. Clamped, so the edge does not repeat when a
    // rotated placement samples fractionally outside.
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return texture;
  }

  private release(key: string): void {
    const entry = this.entries.get(key);
    if (entry?.texture) this.gl.deleteTexture(entry.texture);
    this.entries.delete(key);
  }

  dispose(): void {
    for (const key of [...this.entries.keys()]) this.release(key);
    this.warpMeshes.clear();
    this.warnings.forgetAll();
  }
}
