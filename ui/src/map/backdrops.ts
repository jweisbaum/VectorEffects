/**
 * Textures for the backdrops: charts, map tiles and GIS layers
 * (spec.md 4.11).
 *
 * All three arrive through the same `ve-tile://` scheme the field tiles use,
 * as PNGs the webview decodes for free, painted in Rust onto the
 * application's own tile grid. So the map draws them exactly as it draws a
 * tile of field — same geometry, same projection paths — and knows nothing
 * about what S-57 or Web Mercator are.
 *
 * An address carries a token that changes when its source does: the chart
 * directory's path, or the document revision for a GIS layer. That keeps
 * every address immutable, which is what lets the webview cache them for a
 * year and what makes changing the directory take effect at once.
 */

import type { VisibleTile } from "./camera";

/** One backdrop to draw, under everything, in the order given. */
export interface BackdropDraw {
  /** Its addresses' prefix, which is also its identity for the cache. */
  readonly key: string;
  /** The texture per visible tile, or null for one not here yet. */
  readonly textures: ReadonlyArray<{ tile: VisibleTile; texture: WebGLTexture | null }>;
  /** How strongly it shows, 0 to 1. */
  readonly opacity: number;
  /**
   * Whether it stands in for the built-in basemap rather than sitting over
   * it. Only the map tiles do: they are a map of the whole world, and
   * drawing the app's own land over them would be two coastlines. A chart
   * covers the stretch of coast it was published for and nothing else, so
   * hiding the basemap for one leaves a black globe around it.
   */
  readonly replacesBase: boolean;
}

/** What the backdrops are asked to draw. */
export interface BackdropRequest {
  /** The chart directory, when *Charts* is on and a directory is chosen. */
  charts?: { token: number } | undefined;
  /** OpenStreetMap tiles, when the box is on. */
  osm?: boolean | undefined;
  /** One entry per visible GIS layer, bottom of the stack first. */
  gis?: ReadonlyArray<{ layer: number; token: number }> | undefined;
}

/** How a fetch is doing. */
type Status = "pending" | "ready" | "blank" | "failed";

interface Entry {
  texture: WebGLTexture | null;
  status: Status;
}

/**
 * The address of one tile. The prefix is everything before `z/x/y`, which is
 * what the frontend treats as one opaque token — the same rule the field
 * tiles follow, so a cache keyed on it stays one-to-one with a source.
 */
export function backdropKey(kind: string, token: number, layer?: number): string {
  return layer === undefined
    ? `backdrop/${kind}/${token}`
    : `backdrop/${kind}/${token}/${layer}`;
}

/**
 * Fetches and holds one texture per backdrop tile.
 *
 * Bounded, and evicted by what was asked for last: panning a chart across a
 * continent would otherwise hold every tile it crossed.
 */
export class BackdropCache {
  private readonly gl: WebGL2RenderingContext;
  private readonly baseUrl: string;
  private readonly entries = new Map<string, Entry>();
  /** The keys asked for on the last frame, newest last. */
  private recent: string[] = [];
  /** Called when a fetch completes, so the caller can redraw. */
  onChange: (() => void) | null = null;
  /** How many tiles are held before the least recently wanted is dropped. */
  private readonly limit: number;

  constructor(gl: WebGL2RenderingContext, baseUrl: string, limit = 512) {
    this.gl = gl;
    this.baseUrl = baseUrl;
    this.limit = limit;
  }

  /**
   * What to draw for this request, fetching what is not yet here.
   *
   * Returns an entry per tile whether or not its texture has arrived: the
   * renderer skips the ones that have not and redraws when `onChange`
   * fires, so a slow chart never blanks what is already on screen.
   */
  draws(request: BackdropRequest, tiles: readonly VisibleTile[]): BackdropDraw[] {
    const out: BackdropDraw[] = [];
    const wanted = new Set<string>();

    const add = (key: string, opacity: number, replacesBase = false) => {
      const textures = tiles.map((tile) => {
        const address = `${key}/${tile.z}/${tile.x}/${tile.y}.png`;
        wanted.add(address);
        return { tile, texture: this.texture(address) };
      });
      out.push({ key, textures, opacity, replacesBase });
    };

    // The map tiles replace the basemap, so they go first and the chart
    // over them: a chart is the thing being navigated by.
    if (request.osm) add(backdropKey("osm", 1), 1, true);
    if (request.charts) add(backdropKey("chart", request.charts.token), 1);
    for (const { layer, token } of request.gis ?? []) {
      add(backdropKey("gis", token, layer), 1);
    }

    this.retain(wanted);
    return out;
  }

  /** Drops everything not in `wanted`, oldest first, down to the limit. */
  private retain(wanted: ReadonlySet<string>): void {
    this.recent = [
      ...this.recent.filter((key) => !wanted.has(key)),
      ...wanted,
    ];
    while (this.recent.length > this.limit) {
      const oldest = this.recent.shift();
      if (oldest !== undefined && !wanted.has(oldest)) this.release(oldest);
    }
  }

  private texture(address: string): WebGLTexture | null {
    const existing = this.entries.get(address);
    if (existing) return existing.texture;
    const entry: Entry = { texture: null, status: "pending" };
    this.entries.set(address, entry);
    void this.fetch(address, entry);
    return null;
  }

  private async fetch(address: string, entry: Entry): Promise<void> {
    try {
      const response = await fetch(`${this.baseUrl}${address}`);
      // 204: nothing there. Most of a viewport is not covered by a chart
      // directory, so this is the ordinary answer and not a failure.
      if (response.status === 204) {
        entry.status = "blank";
        return;
      }
      if (!response.ok) throw new Error(`status ${response.status}`);
      const blob = await response.blob();
      if (blob.size === 0) {
        entry.status = "blank";
        return;
      }
      const bitmap = await createImageBitmap(blob);
      // The entry may have been released while the fetch was in flight.
      if (this.entries.get(address) !== entry) {
        bitmap.close();
        return;
      }
      entry.texture = this.upload(bitmap);
      entry.status = entry.texture ? "ready" : "failed";
      bitmap.close();
      if (entry.status === "ready") this.onChange?.();
    } catch {
      entry.status = "failed";
    }
  }

  private upload(bitmap: ImageBitmap): WebGLTexture | null {
    const gl = this.gl;
    const texture = gl.createTexture();
    if (!texture) return null;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    // Clamped, so a tile does not sample its neighbour across the seam, and
    // linear, because a backdrop is a picture: unlike a field tile, nothing
    // is packed across byte pairs here.
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, bitmap);
    return texture;
  }

  private release(address: string): void {
    const entry = this.entries.get(address);
    if (!entry) return;
    if (entry.texture) this.gl.deleteTexture(entry.texture);
    this.entries.delete(address);
  }

  /** How many tiles are held, for a test. */
  get held(): number {
    return this.entries.size;
  }

  dispose(): void {
    for (const address of [...this.entries.keys()]) this.release(address);
    this.recent = [];
  }
}
