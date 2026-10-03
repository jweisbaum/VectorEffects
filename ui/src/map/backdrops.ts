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
import { sstTileRange } from "./sstRamp";

/** One backdrop to draw, under everything, in the order given. */
export interface BackdropDraw {
  /** Its addresses' prefix, which is also its identity for the cache. */
  readonly key: string;
  /**
   * The texture per visible tile, or null for one not here yet; for a
   * temperature backdrop, also the tile's coldest and warmest water, °C.
   */
  readonly textures: ReadonlyArray<{
    tile: VisibleTile;
    texture: WebGLTexture | null;
    range?: readonly [number, number] | null;
  }>;
  /**
   * Whether its tiles are temperatures to colour rather than a picture
   * (spec.md 4.10): drawn by their own shader, on the ramp in force.
   */
  readonly temperature?: boolean;
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
  gis?: ReadonlyArray<LayerBackdrop> | undefined;
  /**
   * One entry per visible sea-surface temperature layer with a day at this
   * step, bottom of the stack first (spec.md 4.10, M93). The token is that
   * day's, so the step is already in it.
   */
  sst?: ReadonlyArray<LayerBackdrop> | undefined;
}

/** A backdrop that belongs to a layer of the document. */
export interface LayerBackdrop {
  readonly layer: number;
  readonly token: number;
  /**
   * The layer's position in the stack, bottom 0. GIS and temperature layers
   * arrive in two lists and are drawn in one stack, so this is what
   * interleaves them; without it each list keeps its own order, GIS first.
   */
  readonly stack?: number | undefined;
}

/** How a fetch is doing. */
type Status = "pending" | "ready" | "blank" | "failed";

interface Entry {
  texture: WebGLTexture | null;
  status: Status;
  /** A temperature tile's coldest and warmest water, °C. */
  range: [number, number] | null;
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
 * The layers' backdrops in the order they are drawn: the stack's, bottom
 * first. A stable sort, so entries with no position keep the order given.
 */
export function layerBackdrops(
  request: BackdropRequest,
): Array<{ kind: "gis" | "sst"; layer: number; token: number }> {
  const all = [
    ...(request.gis ?? []).map((entry) => ({ kind: "gis" as const, ...entry })),
    ...(request.sst ?? []).map((entry) => ({ kind: "sst" as const, ...entry })),
  ];
  return all
    .map((entry, index) => ({ entry, index }))
    .sort(
      (a, b) =>
        (a.entry.stack ?? Number.POSITIVE_INFINITY) - (b.entry.stack ?? Number.POSITIVE_INFINITY) ||
        a.index - b.index,
    )
    .map(({ entry }) => ({ kind: entry.kind, layer: entry.layer, token: entry.token }));
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

    const add = (key: string, opacity: number, replacesBase = false, temperature = false) => {
      const textures = tiles.map((tile) => {
        const address = `${key}/${tile.z}/${tile.x}/${tile.y}.${temperature ? "bin" : "png"}`;
        wanted.add(address);
        const texture = this.texture(address, temperature);
        return { tile, texture, range: this.entries.get(address)?.range ?? null };
      });
      out.push({ key, textures, opacity, replacesBase, temperature });
    };

    // The map tiles replace the basemap, so they go first and the chart
    // over them: a chart is the thing being navigated by.
    if (request.osm) add(backdropKey("osm", 1), 1, true);
    if (request.charts) add(backdropKey("chart", request.charts.token), 1);
    for (const { kind, layer, token } of layerBackdrops(request)) {
      add(backdropKey(kind, token, layer), 1, false, kind === "sst");
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

  private texture(address: string, temperature = false): WebGLTexture | null {
    const existing = this.entries.get(address);
    if (existing) return existing.texture;
    const entry: Entry = { texture: null, status: "pending", range: null };
    this.entries.set(address, entry);
    void this.fetch(address, entry, temperature);
    return null;
  }

  private async fetch(address: string, entry: Entry, temperature: boolean): Promise<void> {
    try {
      const response = await fetch(`${this.baseUrl}${address}`);
      // 204: nothing there. Most of a viewport is not covered by a chart
      // directory, so this is the ordinary answer and not a failure.
      if (response.status === 204) {
        entry.status = "blank";
        return;
      }
      if (!response.ok) throw new Error(`status ${response.status}`);
      if (temperature) {
        const bytes = new Uint8Array(await response.arrayBuffer());
        if (this.entries.get(address) !== entry) return;
        entry.texture = this.uploadTemperatures(bytes);
        entry.range = sstTileRange(bytes);
        entry.status = entry.texture ? "ready" : "failed";
        if (entry.status === "ready") this.onChange?.();
        return;
      }
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

  /**
   * A temperature tile: 256 × 256 pixels of sixteen-bit values, uploaded as
   * they came. NEAREST, as a field tile is, because a value split across two
   * bytes blended by the hardware is no value at all; the shader blends the
   * unpacked temperatures itself.
   */
  private uploadTemperatures(bytes: Uint8Array): WebGLTexture | null {
    if (bytes.byteLength !== 256 * 256 * 4) return null;
    const gl = this.gl;
    const texture = gl.createTexture();
    if (!texture) return null;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 256, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
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
