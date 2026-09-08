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
import type { ImageDraw } from "./renderer";

/** How a fetch is doing. */
type Status = "pending" | "ready" | "failed";

interface Entry {
  texture: WebGLTexture | null;
  status: Status;
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
    views: readonly ImageLayerView[],
    over: ReadonlySet<number> = new Set(),
  ): ImageDraw[] {
    const wanted = new Set<string>();
    const out: ImageDraw[] = [];
    for (const view of views) {
      if (!view.loaded || view.width === 0 || view.height === 0) continue;
      const key = `image/${token}/${view.layer}/${this.maxEdge}`;
      wanted.add(key);
      out.push({
        layer: view.layer,
        texture: this.texture(key),
        opacity: view.opacity,
        over: over.has(view.layer),
        ...placeVectors(view),
      });
    }
    // An image that is gone — deleted, or at an older revision — takes its
    // texture with it. There is no LRU here because there is no pressure: a
    // project has a handful of image layers, not a pyramid of tiles.
    for (const key of [...this.entries.keys()]) {
      if (!wanted.has(key)) this.release(key);
    }
    return out;
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
  }
}
