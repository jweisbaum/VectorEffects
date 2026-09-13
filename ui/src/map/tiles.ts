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
import { playbackCount, playbackTime } from "../timeline/metrics";
import { glyphCoverage } from "./glyphPlacement";

/** How a tile is doing. */
export type TileStatus = "ready" | "pending" | "failed";

interface Entry {
  texture: WebGLTexture | null;
  coverage: Uint8Array | null;
  windCoverage: Uint8Array | null;
  status: TileStatus;
  /**
   * The tile's speed range of each kind as 14-bit fractions of full scale,
   * read once at upload (`tileSpeedRange`), or null for a tile with no
   * field in it.
   */
  range: TileRanges | null;
  attempts: number;
  retryAt: number;
  background: boolean;
}

interface FetchJob {
  frame: string;
  address: string;
  key: string;
  entry: Entry;
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

/** Metadata is cheap; keep a full 240-step run plus scoped editing frames. */
const FRAMES_KEPT = 256;
// WebKit's custom-scheme requests spend most of their time outside the
// JavaScript task queue. Keep enough requests in flight to fill that gap;
// uploads remain independently budgeted below so this does not block draws.
const FETCHES = 64;
const RESOLVERS = 2;

/** One tile's texture: 256 x 256 RGBA8, matching `TILE_BYTES` in `ve-render`. */
export const TILE_BYTES = 256 * 256 * 4;

/**
 * The cache floor, in tiles: room for a viewport at several steps.
 *
 * A viewport is at most 192 tiles (`visibleTiles`'s budget), so this is four
 * of them — the frame being drawn, the frame held behind it, and the two
 * warmed ahead of the playhead.
 */
export const BASE_CAPACITY = 768;

/**
 * The most texture memory the tile cache may hold.
 *
 * A whole timeline of a large viewport is unbounded — 240 steps of 170 tiles
 * is ten gigabytes — so the reservation stops here. One gigabyte is chosen to
 * fit a discrete GPU's memory alongside everything else the map draws; past
 * this a long timeline simply refetches, which is slow rather than broken.
 */
export const MAX_CACHE_BYTES = 1024 * 1024 * 1024;

/** [`MAX_CACHE_BYTES`] as a tile count. */
export const MAX_CAPACITY = Math.floor(MAX_CACHE_BYTES / TILE_BYTES);

/** Tracks fetched tiles and their textures. */
export class TileCache {
  private readonly gl: WebGL2RenderingContext;
  private readonly baseUrl: string;
  /** Raised and lowered by `reserve` to fit the open timeline. */
  private limit: number;
  /** Textures by key. Insertion-ordered, which makes it an LRU when re-inserted on access. */
  private readonly entries = new Map<string, Entry>();
  /** Each frame's tiles' keys, as far as they have been resolved. */
  private readonly frames = new Map<string, Map<string, string>>();
  /** Tiles asked about since the last resolution, by frame. */
  private readonly pending = new Map<string, Map<string, TileAddress>>();
  /** Frames whose resolution is in flight, with the tiles it covers. */
  private readonly resolving = new Map<string, Set<string>>();
  private flushScheduled = false;
  private disposed = false;
  private frameLimit = FRAMES_KEPT;
  private readonly resolutionFailures = new Map<string, { attempts: number; retryAt: number }>();
  private readonly background = new Set<string>();
  private protectedFrames: readonly string[] = [];
  private protectedTiles: ReadonlyArray<TileAddress> = [];
  private preparationTiles = new Set<string>();
  private pinned = new Set<string>();
  private jobs: FetchJob[] = [];
  private activeFetches = 0;
  private uploads: Array<{ job: FetchJob; bytes: Uint8Array }> = [];
  private uploadTimer: ReturnType<typeof setTimeout> | null = null;
  private uploadScheduled = false;
  private readonly uploadChannel: MessageChannel | null;
  private readonly preparingAt = new Map<string, number>();
  /** Whole-frame latency, including queueing, resolution, transfer, and upload. */
  preparationMs = 250;
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
   * `limit` is in tiles of [`TILE_BYTES`] each, and is raised to fit the open
   * timeline by [`reserve`].
   */
  constructor(gl: WebGL2RenderingContext, baseUrl: string, limit = BASE_CAPACITY) {
    this.gl = gl;
    this.baseUrl = baseUrl;
    this.limit = limit;
    // A posted task yields to drawing without the nested-timer clamp that
    // made two-tile timer batches cap streaming at roughly two frames/s.
    this.uploadChannel = typeof window !== "undefined" && typeof MessageChannel !== "undefined" ? new MessageChannel() : null;
    if (this.uploadChannel) this.uploadChannel.port1.onmessage = () => this.drainUploads();
  }

  /** How many tiles the cache will hold before it evicts. */
  get capacity(): number {
    return this.limit;
  }

  /**
   * Sizes the cache to hold a whole timeline of one viewport.
   *
   * Playback advances only into a step every tile of which is resident, and
   * those tiles must be fetched and uploaded however long ago the backend
   * rendered them — pre-rendering removes the render cost, never the crossing
   * cost. So a loop that does not fit in the cache pays that crossing on every
   * lap and can never reach the set rate; one that fits pays it once and then
   * plays as fast as it is asked to.
   *
   * Bounded by [`MAX_CACHE_BYTES`], because the product is unbounded: 240
   * steps of a 170-tile viewport is ten gigabytes of texture. Floored at
   * [`BASE_CAPACITY`] so a short timeline does not shrink the cache below what
   * panning around a single step wants.
   */
  reserve(steps: number, tilesPerFrame: number): void {
    this.frameLimit = Math.max(FRAMES_KEPT, Math.ceil(steps) * 3 + 8);
    // Leave room for the held frame and the editor's layer scope as well.
    const wanted = (Math.max(0, Math.ceil(steps)) + 2) * Math.max(0, Math.ceil(tilesPerFrame));
    const limit = Math.min(MAX_CAPACITY, Math.max(BASE_CAPACITY, wanted));
    if (limit === this.limit) return;
    this.limit = limit;
    // Shrinking has to take effect now rather than at the next miss, or the
    // memory the smaller viewport released is held until something else
    // happens to fetch.
    this.evict();
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
    return this.request(frame, z, x, y, false);
  }

  private request(frame: string, z: number, x: number, y: number, background: boolean): WebGLTexture | null {
    if (this.disposed) return null;
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
      if (existing.status === "failed" && existing.attempts < 3 && performance.now() >= existing.retryAt) {
        existing.status = "pending";
        this.enqueue(frame, z, x, y, key, existing, background);
      }
      if (!background) {
        existing.background = false;
        if (existing.status === "pending") {
          const at = this.jobs.findIndex((job) => job.entry === existing);
          if (at > 0) this.jobs.unshift(this.jobs.splice(at, 1)[0]!);
        }
      }
      return existing.texture;
    }

    const entry: Entry = { texture: null, coverage: null, windCoverage: null, status: "pending", range: null, attempts: 0, retryAt: 0, background };
    this.entries.set(key, entry);
    this.enqueue(frame, z, x, y, key, entry, background);
    this.evict();
    return null;
  }

  private enqueue(frame: string, z: number, x: number, y: number, key: string, entry: Entry, background: boolean): void {
    const job = { frame, address: `${frame}/${TileCache.tileKey(z, x, y)}`, key, entry };
    if (background) this.jobs.push(job);
    else this.jobs.unshift(job);
    this.pumpFetches();
  }

  private pumpFetches(): void {
    while (!this.disposed && this.activeFetches < FETCHES && this.jobs.length > 0) {
      const job = this.jobs.shift()!;
      if (this.entries.get(job.key) !== job.entry) continue;
      this.activeFetches++;
      void this.fetch(job);
    }
  }

  /** Protect the displayed frame and playback window before starting more work. */
  protect(frames: readonly string[], tiles: ReadonlyArray<TileAddress>): void {
    this.protectedFrames = frames;
    this.protectedTiles = tiles;
    this.refreshPins();
    this.evict();
  }

  private refreshPins(): void {
    this.pinned = new Set();
    for (const frame of this.protectedFrames) for (const tile of this.protectedTiles) {
      const key = this.keyOf(frame, tile.z, tile.x, tile.y);
      if (key !== undefined) this.pinned.add(key);
    }
  }

  /** A bounded set of frames to prepare while paused as well as while playing. */
  prepare(frames: readonly string[], tiles: ReadonlyArray<TileAddress>): number {
    const before = new Set(this.background);
    this.background.clear();
    for (const frame of frames) this.background.add(frame);
    for (const frame of this.preparingAt.keys()) if (!this.background.has(frame)) this.preparingAt.delete(frame);
    this.preparationTiles = new Set(tiles.map((tile) => TileCache.tileKey(tile.z, tile.x, tile.y)));
    for (const [frame, wanted] of this.pending) {
      if (!before.has(frame)) continue;
      for (const key of wanted.keys()) {
        if (!this.background.has(frame) || !this.preparationTiles.has(key)) wanted.delete(key);
      }
      if (!wanted.size) this.pending.delete(frame);
    }
    this.jobs = this.jobs.filter((job) => {
      if (!job.entry.background || (this.background.has(job.frame) && this.preparationTiles.has(job.address.slice(job.frame.length + 1)))) return true;
      if (this.entries.get(job.key) === job.entry) this.entries.delete(job.key);
      return false;
    });
    let ready = 0;
    for (const frame of frames) {
      let resident = 0;
      for (const tile of tiles) {
        if (this.request(frame, tile.z, tile.x, tile.y, true)) resident++;
      }
      if (resident === tiles.length) {
        ready++;
        const started = this.preparingAt.get(frame);
        if (started !== undefined) {
          const elapsed = performance.now() - started;
          this.preparationMs = Math.max(100, this.preparationMs * 0.75 + elapsed * 0.25);
          playbackTime("prepareFrameMs", elapsed);
          this.preparingAt.delete(frame);
        }
      } else if (!this.preparingAt.has(frame)) {
        this.preparingAt.set(frame, performance.now());
      }
    }
    return ready;
  }

  /**
   * Whether the frame's *key* for this tile is still unknown (M70).
   *
   * The difference between a lookup and a fetch, which `get` returning null
   * cannot express: before a frame is resolved nothing is known about the
   * tile, and after it is, a missing texture means the content genuinely
   * changed and is on its way. The caller dims for the second and not the
   * first — see `tileSource`.
   */
  unresolved(frame: string, z: number, x: number, y: number): boolean {
    return this.keyOf(frame, z, x, y) === undefined;
  }

  /** The texture for a tile if it is resident, fetching nothing. */
  peek(frame: string, z: number, x: number, y: number): WebGLTexture | null {
    const key = this.keyOf(frame, z, x, y);
    return key === undefined ? null : (this.entries.get(key)?.texture ?? null);
  }

  /** Coverage of the resident texture, for filling gaps in the glyph lattice. */
  coverageOf(frame: string, z: number, x: number, y: number): Uint8Array | null {
    const key = this.keyOf(frame, z, x, y);
    return key === undefined ? null : (this.entries.get(key)?.coverage ?? null);
  }

  /** The subset of covered texels whose visible field is wind. */
  windCoverageOf(frame: string, z: number, x: number, y: number): Uint8Array | null {
    const key = this.keyOf(frame, z, x, y);
    return key === undefined ? null : (this.entries.get(key)?.windCoverage ?? null);
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
    const failure = this.resolutionFailures.get(frame);
    if (failure && (failure.attempts >= 3 || performance.now() < failure.retryAt)) return;
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
    if (!resolver || this.disposed) return;
    for (const [frame, tiles] of this.pending) {
      if (this.resolving.size >= RESOLVERS) break;
      if (this.resolving.has(frame)) continue;
      this.pending.delete(frame);
      const list = [...tiles.values()];
      const covered = this.resolving.get(frame) ?? new Set<string>();
      for (const tile of list) covered.add(TileCache.tileKey(tile.z, tile.x, tile.y));
      this.resolving.set(frame, covered);
      void this.resolve(resolver, frame, list);
    }
  }

  private async resolve(
    resolver: KeyResolver,
    frame: string,
    tiles: TileAddress[],
  ): Promise<void> {
    const started = performance.now();
    const background = this.background.has(frame);
    try {
      playbackCount("keyResolutions");
      const keys = await resolver(frame, tiles);
      if (this.disposed) return;
      this.resolutionFailures.delete(frame);
      let map = this.frames.get(frame);
      if (!map) {
        map = new Map();
        this.frames.set(frame, map);
        // The oldest frames' maps go; their textures stay by key.
        while (this.frames.size > this.frameLimit) {
          const oldest = [...this.frames.keys()].find((key) => !this.protectedFrames.includes(key));
          if (oldest === undefined) break;
          this.frames.delete(oldest);
          this.resolutionFailures.delete(oldest);
        }
      }
      tiles.forEach((tile, index) => {
        const key = keys[index];
        if (key) map.set(TileCache.tileKey(tile.z, tile.x, tile.y), key);
      });
      this.refreshPins();
      // Resolution drives the next stage immediately, even with playback paused.
      for (const tile of tiles) {
        if (background && (!this.background.has(frame) || !this.preparationTiles.has(TileCache.tileKey(tile.z, tile.x, tile.y)))) continue;
        this.request(frame, tile.z, tile.x, tile.y, background);
      }
    } catch (error) {
      if (this.disposed) return;
      // The revision moved on before it was asked about — an edit landed
      // between the draw and the request. The next draw asks with the new one.
      console.debug(`tile keys for ${frame} unavailable: ${String(error)}`);
      const attempts = (this.resolutionFailures.get(frame)?.attempts ?? 0) + 1;
      this.resolutionFailures.set(frame, { attempts, retryAt: performance.now() + 250 * 4 ** (attempts - 1) });
    } finally {
      const covered = this.resolving.get(frame);
      if (covered) {
        for (const tile of tiles) covered.delete(TileCache.tileKey(tile.z, tile.x, tile.y));
        if (covered.size === 0) this.resolving.delete(frame);
      }
      playbackTime("keysMs", performance.now() - started);
      this.flush();
    }
    if (!this.disposed) this.onChange?.();
  }

  private async fetch(job: FetchJob): Promise<void> {
    const { address, key, entry } = job;
    const started = performance.now();
    let uploading = false;
    entry.attempts++;
    try {
      playbackCount("tileFetches");
      const response = await fetch(`${this.baseUrl}${address}`);
      if (!response.ok) throw new Error(`status ${response.status}`);
      const bytes = new Uint8Array(await response.arrayBuffer());

      // The entry may have been evicted while the fetch was in flight.
      if (this.disposed || this.entries.get(key) !== entry) return;
      if (entry.background && (!this.background.has(job.frame) || !this.preparationTiles.has(address.slice(job.frame.length + 1)))) {
        this.entries.delete(key);
        return;
      }
      playbackCount("tileBytes", bytes.length);
      this.uploads.push({ job, bytes });
      uploading = true;
      this.scheduleUploads();
    } catch (error) {
      entry.status = "failed";
      entry.retryAt = performance.now() + 250 * 4 ** (entry.attempts - 1);
      const message = `tile ${address} failed: ${error instanceof Error ? error.message : String(error)}`;
      console.warn(message);
      this.onError?.(message);
    } finally {
      const elapsed = performance.now() - started;
      playbackTime("fetchMs", elapsed);
      if (!uploading) {
        this.activeFetches--;
        this.pumpFetches();
        if (!this.disposed) this.onChange?.();
      }
    }
  }

  private scheduleUploads(): void {
    if (this.uploadScheduled || this.disposed) return;
    this.uploadScheduled = true;
    if (this.uploadChannel) this.uploadChannel.port2.postMessage(null);
    else this.uploadTimer = setTimeout(() => this.drainUploads(), 0);
  }

  private drainUploads(): void {
    this.uploadScheduled = false;
    this.uploadTimer = null;
    if (this.disposed) return;
    const started = performance.now();
    while (this.uploads.length && performance.now() - started < 4) {
      const { job, bytes } = this.uploads.shift()!;
      const { entry, key } = job;
      const wanted = !entry.background || (this.background.has(job.frame) && this.preparationTiles.has(job.address.slice(job.frame.length + 1)));
      if (this.entries.get(key) === entry && wanted) {
        const at = performance.now();
        try {
          entry.texture = this.upload(bytes);
          if (!entry.texture) throw new Error("texture upload failed");
          const ranges = tileSpeedRange(bytes);
          entry.range = hasField(ranges) ? ranges : null;
          entry.coverage = glyphCoverage(bytes);
          entry.windCoverage = glyphCoverage(bytes, true);
          entry.status = "ready";
          playbackCount("tileUploads");
        } catch (error) {
          entry.status = "failed";
          entry.retryAt = performance.now() + 1000;
          this.onError?.(String(error));
        }
        playbackTime("uploadAndRangeMs", performance.now() - at);
      } else if (this.entries.get(key) === entry) {
        this.entries.delete(key);
      }
      this.activeFetches--;
    }
    this.pumpFetches();
    this.onChange?.();
    if (this.uploads.length) this.scheduleUploads();
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
      const oldest = [...this.entries.keys()].find((key) => !this.pinned.has(key));
      if (oldest === undefined) break;
      const entry = this.entries.get(oldest);
      if (entry?.texture) this.gl.deleteTexture(entry.texture);
      this.entries.delete(oldest);
      playbackCount("tileEvictions");
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
    this.disposed = true;
    if (this.uploadTimer !== null) clearTimeout(this.uploadTimer);
    this.uploadChannel?.port1.close();
    this.uploadChannel?.port2.close();
    this.jobs = [];
    this.uploads = [];
    for (const entry of this.entries.values()) {
      if (entry.texture) this.gl.deleteTexture(entry.texture);
    }
    this.entries.clear();
    this.frames.clear();
    this.pending.clear();
    this.resolving.clear();
  }
}

/** Where a tile on screen is coming from, and whether it is marked stale. */
export type TileSource = "frame" | "held" | "held-stale";

/**
 * Which of the three a tile should be drawn from (M70).
 *
 * An edit re-addresses every tile on the map — the address carries the
 * revision — so for one round trip after every stroke *no* tile of the new
 * frame has a key yet. Dimming on that produced a map-wide flash on every
 * edit, thousands of kilometres from the edit itself, which then resolved
 * back to full brightness in rectangular batches as the keys landed.
 *
 * The three states are different things and only one of them is stale:
 *
 * - the frame's own texture is resident — draw it;
 * - the key is **not yet known** — draw the held frame plainly. Nothing says
 *   this tile changed, and for the overwhelming majority it has not: the key
 *   is about to come back as the one already on screen. Showing the pixels it
 *   is about to be confirmed as is not a lie, and it is what makes an edit far
 *   away cost nothing visible;
 * - the key **is** known and differs, so the tile is fetching — draw the held
 *   frame dimmed. Here the content really has changed and the dim is telling
 *   the truth.
 */
/**
 * The field is named for [`TileCache.unresolved`] and taken unnegated, so the
 * call site reads `unresolved: cache.unresolved(...)`. A `keyResolved` taking
 * the opposite sense would be one stray `!` away from dimming exactly when it
 * should not, and that mistake is invisible to a test of this function.
 */
export function tileSource(state: {
  hasTexture: boolean;
  hasHeldFrame: boolean;
  unresolved: boolean;
}): TileSource {
  if (state.hasTexture || !state.hasHeldFrame) return "frame";
  return state.unresolved ? "held" : "held-stale";
}

/** How far a live edit reaches while it is being drawn. */
export type EditScope = "everywhere" | "layer" | "nowhere";

/**
 * How far a gesture in progress should reach (M72).
 *
 * - Nothing asked for a scope: the tool is not aimed at one layer, so the
 *   gesture applies **everywhere**, which is what an unscoped tool means.
 * - A scope was asked for and its tile is resident: the gesture applies to the
 *   pixels that **layer** contributed, found by comparing the two tiles.
 * - A scope was asked for and has not arrived: **nowhere**, until it does.
 *
 * The third case is the one that was wrong. It fell back to `everywhere`, on
 * the grounds that showing the gesture over every layer for a frame or two
 * beat showing it over none — and the frame or two turned out to be the whole
 * stroke, because nothing fetched the scope once the button was down. An
 * intensity aimed at a current layer visibly intensified the wind beneath it
 * for the length of every stroke.
 *
 * Showing nothing briefly is a delay. Showing an edit to a layer the user did
 * not aim at is a lie about what the tool does, and it is the one thing
 * scoping exists to prevent — so an asked-for scope is never dropped.
 */
export function editScope(asked: boolean, resident: boolean): EditScope {
  if (!asked) return "everywhere";
  return resident ? "layer" : "nowhere";
}
