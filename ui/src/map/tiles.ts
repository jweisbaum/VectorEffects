/**
 * Tile fetching and GPU residency.
 *
 * Tiles come from the `ve-tile://` scheme rather than an IPC command, so the
 * browser's own cache and request pipelining apply and a quarter-megabyte of
 * pixels never crosses the IPC channel.
 *
 * A tile URL names a specific field, so it never changes meaning: entries can
 * be cached indefinitely and evicted purely on pressure.
 */

/** How a tile is doing. */
export type TileStatus = "ready" | "pending" | "failed";

interface Entry {
  texture: WebGLTexture | null;
  status: TileStatus;
  /**
   * The tile's speed range as 16-bit fractions of full scale, read once at
   * upload (`tileSpeedRange`), or null for a tile with no field in it.
   */
  range: [number, number] | null;
}

import { tileSpeedRange } from "./tileRange";

/** Tracks fetched tiles and their textures. */
export class TileCache {
  private readonly gl: WebGL2RenderingContext;
  private readonly baseUrl: string;
  private readonly limit: number;
  /** Insertion-ordered, which makes it an LRU when re-inserted on access. */
  private readonly entries = new Map<string, Entry>();
  /** Called when a fetch completes, so the caller can redraw. */
  onChange: (() => void) | null = null;
  /** Called when a fetch fails, so the failure reaches the application log. */
  onError: ((message: string) => void) | null = null;

  /**
   * `limit` is in tiles of a quarter megabyte each. Playback shows a viewport
   * at every step in turn and loops, so the cache wants room for a viewport
   * (up to 192 tiles) at several steps, or a loop refetches every step every
   * time round. 768 is 192 MB at the very largest viewport; typical viewports
   * are a quarter of that.
   */
  constructor(gl: WebGL2RenderingContext, baseUrl: string, limit = 768) {
    this.gl = gl;
    this.baseUrl = baseUrl;
    this.limit = limit;
  }

  private static key(frame: string, z: number, x: number, y: number): string {
    return `${frame}/${z}/${x}/${y}`;
  }

  /**
   * The texture for a tile, fetching it if absent.
   *
   * Returns null while a fetch is in flight; the caller draws whatever it has
   * and redraws when `onChange` fires, so panning never blanks the map.
   */
  get(frame: string, z: number, x: number, y: number): WebGLTexture | null {
    const key = TileCache.key(frame, z, x, y);
    const existing = this.entries.get(key);
    if (existing) {
      // Re-insert to mark as recently used.
      this.entries.delete(key);
      this.entries.set(key, existing);
      return existing.texture;
    }

    const entry: Entry = { texture: null, status: "pending", range: null };
    this.entries.set(key, entry);
    void this.fetch(key, entry);
    this.evict();
    return null;
  }

  /** The texture for a tile if it is resident, fetching nothing. */
  peek(frame: string, z: number, x: number, y: number): WebGLTexture | null {
    return this.entries.get(TileCache.key(frame, z, x, y))?.texture ?? null;
  }

  /**
   * A resident tile's speed range as 16-bit fractions of full scale, or null
   * when it is not resident or holds no field. Fetches nothing.
   */
  rangeOf(frame: string, z: number, x: number, y: number): [number, number] | null {
    return this.entries.get(TileCache.key(frame, z, x, y))?.range ?? null;
  }

  /** How many of a frame's tiles are resident, fetching nothing. */
  residentCount(frame: string, tiles: ReadonlyArray<{ z: number; x: number; y: number }>): number {
    let count = 0;
    for (const tile of tiles) {
      if (this.peek(frame, tile.z, tile.x, tile.y)) count++;
    }
    return count;
  }

  /**
   * Starts fetching a frame's tiles, and says whether they are all resident.
   *
   * Playback asks this of the step it is about to show: the backend can hold a
   * rendered tile the GPU does not yet, and advancing on the backend's word
   * alone shows the previous step under the new one for a frame (spec.md 9.4).
   */
  prefetch(frame: string, tiles: ReadonlyArray<{ z: number; x: number; y: number }>): boolean {
    let resident = 0;
    for (const tile of tiles) {
      if (this.get(frame, tile.z, tile.x, tile.y)) resident++;
    }
    return resident === tiles.length;
  }

  private async fetch(key: string, entry: Entry): Promise<void> {
    try {
      const response = await fetch(`${this.baseUrl}${key}`);
      if (!response.ok) throw new Error(`status ${response.status}`);
      const bytes = new Uint8Array(await response.arrayBuffer());

      // The entry may have been evicted while the fetch was in flight.
      if (!this.entries.has(key)) return;

      entry.texture = this.upload(bytes);
      entry.status = entry.texture ? "ready" : "failed";
      entry.range = entry.texture ? tileSpeedRange(bytes) : null;
    } catch (error) {
      entry.status = "failed";
      const message = `tile ${key} failed: ${error instanceof Error ? error.message : String(error)}`;
      console.warn(message);
      this.onError?.(message);
    }
    this.onChange?.();
  }

  private upload(bytes: Uint8Array): WebGLTexture | null {
    const gl = this.gl;
    const expected = 256 * 256 * 4;
    if (bytes.byteLength !== expected) {
      console.warn(`tile has ${bytes.byteLength} bytes, expected ${expected}`);
      return null;
    }

    const texture = gl.createTexture();
    if (!texture) return null;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 256, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
    // NEAREST is mandatory, not a preference: speed and direction are 16-bit
    // values split across byte pairs, and hardware filtering would blend the
    // high and low bytes independently. Smoothing happens in the shader.
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return texture;
  }

  private evict(): void {
    while (this.entries.size > this.limit) {
      const oldest = this.entries.keys().next();
      if (oldest.done) break;
      const entry = this.entries.get(oldest.value);
      if (entry?.texture) this.gl.deleteTexture(entry.texture);
      this.entries.delete(oldest.value);
    }
  }

  /** Counts by status, for the activity indicator. */
  stats(): { ready: number; pending: number; failed: number } {
    let ready = 0;
    let pending = 0;
    let failed = 0;
    for (const entry of this.entries.values()) {
      if (entry.status === "ready") ready++;
      else if (entry.status === "pending") pending++;
      else failed++;
    }
    return { ready, pending, failed };
  }

  /** Releases every texture. */
  dispose(): void {
    for (const entry of this.entries.values()) {
      if (entry.texture) this.gl.deleteTexture(entry.texture);
    }
    this.entries.clear();
  }
}
