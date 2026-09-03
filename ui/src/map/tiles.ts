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
}

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

  constructor(gl: WebGL2RenderingContext, baseUrl: string, limit = 320) {
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

    const entry: Entry = { texture: null, status: "pending" };
    this.entries.set(key, entry);
    void this.fetch(key, entry);
    this.evict();
    return null;
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
