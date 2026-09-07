/**
 * Tile fetching and GPU residency.
 *
 * Tiles come from the `ve-tile://` scheme rather than an IPC command, so the
 * browser's own cache and request pipelining apply and a quarter-megabyte of
 * pixels never crosses the IPC channel.
 *
 * **A texture is kept by the tile's key, not by its address** (spec.md 7.10,
 * M31). An address is `<revision>/<step>/<z>/<x>/<y>` and an edit changes the
 * revision of every tile on the map; the key is the content hash of the
 * objects that reach the tile, and an edit changes it for the tiles the
 * edited object reaches and no others. So a frame is first *resolved* — the
 * backend is asked for the keys of the tiles in view — and a tile whose key
 * the cache already holds is on screen at once, untouched and undimmed,
 * while only the tiles whose key is new are fetched. A step change on a
 * still scene, likewise, fetches nothing.
 */

import { type TileRanges, hasField, tileSpeedRange } from "./tileRange";

/** How a tile is doing. */
export type TileStatus = "ready" | "pending" | "failed";

interface Entry {
  texture: WebGLTexture | null;
  status: TileStatus;
  /**
   * The tile's speed range of each kind as 14-bit fractions of full scale,
   * read once at upload (`tileSpeedRange`), or null for a tile with no
   * field in it.
   */
  range: TileRanges | null;
}

/** A tile's place in the pyramid. */
export interface TileAddress {
  z: number;
  x: number;
  y: number;
}

/**
 * Asks the backend for the keys of a frame's tiles, in the tiles' order.
 * The frame token is `<revision>/<step>`.
 */
export type KeyResolver = (frame: string, tiles: readonly TileAddress[]) => Promise<string[]>;

/** How many frames' key maps are remembered. Playback loops over a few dozen. */
const FRAMES_KEPT = 96;

/** Tracks fetched tiles and their textures. */
export class TileCache {
  private readonly gl: WebGL2RenderingContext;
  private readonly baseUrl: string;
  private readonly limit: number;
  /** Textures by key. Insertion-ordered, which makes it an LRU when re-inserted on access. */
  private readonly entries = new Map<string, Entry>();
  /** Each frame's tiles' keys, as far as they have been resolved. */
  private readonly frames = new Map<string, Map<string, string>>();
  /** Tiles asked about since the last resolution, by frame. */
  private readonly pending = new Map<string, Map<string, TileAddress>>();
  /** Frames whose resolution is in flight, with the tiles it covers. */
  private readonly resolving = new Map<string, Set<string>>();
  private flushScheduled = false;
  /** Called when a fetch or a resolution completes, so the caller can redraw. */
  onChange: (() => void) | null = null;
  /** Called when a fetch fails, so the failure reaches the application log. */
  onError: ((message: string) => void) | null = null;
  /**
   * How a frame's keys are found. Until one is set, no tile of any frame is
   * resident and none is fetched.
   */
  resolver: KeyResolver | null = null;

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

  private static tileKey(z: number, x: number, y: number): string {
    return `${z}/${x}/${y}`;
  }

  /** The key of a frame's tile, if the frame has been resolved for it. */
  private keyOf(frame: string, z: number, x: number, y: number): string | undefined {
    return this.frames.get(frame)?.get(TileCache.tileKey(z, x, y));
  }

  /**
   * The texture for a tile, fetching it if absent.
   *
   * Returns null while the frame's keys or the tile itself are on their way;
   * the caller draws whatever it has and redraws when `onChange` fires, so
   * panning never blanks the map.
   */
  get(frame: string, z: number, x: number, y: number): WebGLTexture | null {
    const key = this.keyOf(frame, z, x, y);
    if (key === undefined) {
      this.ask(frame, { z, x, y });
      return null;
    }
    const existing = this.entries.get(key);
    if (existing) {
      // Re-insert to mark as recently used.
      this.entries.delete(key);
      this.entries.set(key, existing);
      return existing.texture;
    }

    const entry: Entry = { texture: null, status: "pending", range: null };
    this.entries.set(key, entry);
    void this.fetch(`${frame}/${TileCache.tileKey(z, x, y)}`, key, entry);
    this.evict();
    return null;
  }

  /** The texture for a tile if it is resident, fetching nothing. */
  peek(frame: string, z: number, x: number, y: number): WebGLTexture | null {
    const key = this.keyOf(frame, z, x, y);
    return key === undefined ? null : (this.entries.get(key)?.texture ?? null);
  }

  /**
   * A resident tile's speed ranges as 14-bit fractions of full scale, or
   * null when it is not resident or holds no field. Fetches nothing.
   */
  rangeOf(frame: string, z: number, x: number, y: number): TileRanges | null {
    const key = this.keyOf(frame, z, x, y);
    return key === undefined ? null : (this.entries.get(key)?.range ?? null);
  }

  /** How many of a frame's tiles are resident, fetching nothing. */
  residentCount(frame: string, tiles: ReadonlyArray<TileAddress>): number {
    let count = 0;
    for (const tile of tiles) {
      if (this.peek(frame, tile.z, tile.x, tile.y)) count++;
    }
    return count;
  }

  /**
   * Starts resolving and fetching a frame's tiles, and says whether they are
   * all resident.
   *
   * Playback asks this of the step it is about to show: the backend can hold a
   * rendered tile the GPU does not yet, and advancing on the backend's word
   * alone shows the previous step under the new one for a frame (spec.md 9.4).
   */
  prefetch(frame: string, tiles: ReadonlyArray<TileAddress>): boolean {
    let resident = 0;
    for (const tile of tiles) {
      if (this.get(frame, tile.z, tile.x, tile.y)) resident++;
    }
    return resident === tiles.length;
  }

  /**
   * Notes a tile whose key the frame has not been resolved for, and arranges
   * for one resolution of everything asked this frame. A draw asks for a
   * viewport's tiles one after another; one request answers them all.
   */
  private ask(frame: string, tile: TileAddress): void {
    const tileKey = TileCache.tileKey(tile.z, tile.x, tile.y);
    if (this.resolving.get(frame)?.has(tileKey)) return;
    let tiles = this.pending.get(frame);
    if (!tiles) {
      tiles = new Map();
      this.pending.set(frame, tiles);
    }
    tiles.set(tileKey, tile);
    if (!this.flushScheduled) {
      this.flushScheduled = true;
      queueMicrotask(() => this.flush());
    }
  }

  private flush(): void {
    this.flushScheduled = false;
    const resolver = this.resolver;
    if (!resolver) return;
    for (const [frame, tiles] of this.pending) {
      const list = [...tiles.values()];
      const covered = this.resolving.get(frame) ?? new Set<string>();
      for (const tile of list) covered.add(TileCache.tileKey(tile.z, tile.x, tile.y));
      this.resolving.set(frame, covered);
      void this.resolve(resolver, frame, list);
    }
    this.pending.clear();
  }

  private async resolve(
    resolver: KeyResolver,
    frame: string,
    tiles: TileAddress[],
  ): Promise<void> {
    try {
      const keys = await resolver(frame, tiles);
      let map = this.frames.get(frame);
      if (!map) {
        map = new Map();
        this.frames.set(frame, map);
        // The oldest frames' maps go; their textures stay by key.
        while (this.frames.size > FRAMES_KEPT) {
          const oldest = this.frames.keys().next();
          if (oldest.done) break;
          this.frames.delete(oldest.value);
        }
      }
      tiles.forEach((tile, index) => {
        const key = keys[index];
        if (key) map.set(TileCache.tileKey(tile.z, tile.x, tile.y), key);
      });
    } catch (error) {
      // The revision moved on before it was asked about — an edit landed
      // between the draw and the request. The next draw asks with the new one.
      console.debug(`tile keys for ${frame} unavailable: ${String(error)}`);
    } finally {
      const covered = this.resolving.get(frame);
      if (covered) {
        for (const tile of tiles) covered.delete(TileCache.tileKey(tile.z, tile.x, tile.y));
        if (covered.size === 0) this.resolving.delete(frame);
      }
    }
    this.onChange?.();
  }

  private async fetch(address: string, key: string, entry: Entry): Promise<void> {
    try {
      const response = await fetch(`${this.baseUrl}${address}`);
      if (!response.ok) throw new Error(`status ${response.status}`);
      const bytes = new Uint8Array(await response.arrayBuffer());

      // The entry may have been evicted while the fetch was in flight.
      if (this.entries.get(key) !== entry) return;

      entry.texture = this.upload(bytes);
      entry.status = entry.texture ? "ready" : "failed";
      if (entry.texture) {
        const ranges = tileSpeedRange(bytes);
        entry.range = hasField(ranges) ? ranges : null;
      } else {
        entry.range = null;
      }
    } catch (error) {
      entry.status = "failed";
      const message = `tile ${address} failed: ${error instanceof Error ? error.message : String(error)}`;
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
    this.frames.clear();
    this.pending.clear();
    this.resolving.clear();
  }
}
