/**
 * Where a tile on screen comes from, and when it is marked stale (M70).
 *
 * The regression this pins: an edit re-addresses every tile on the map, since
 * the address carries the revision, so for one round trip after every stroke
 * no tile of the new frame has a key yet. Dimming on that flashed the whole
 * map — thousands of kilometres from the edit — and then resolved back to full
 * brightness in rectangular batches as the keys landed.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

import {
  BASE_CAPACITY,
  editScope,
  MAX_CACHE_BYTES,
  MAX_CAPACITY,
  TILE_BYTES,
  TileCache,
  tileSource,
} from "./tiles";

describe("where a tile is drawn from", () => {
  /** Resident is resident, held frame or not. */
  it("uses the frame's own texture whenever it has one", () => {
    for (const hasHeldFrame of [true, false]) {
      for (const unresolved of [true, false]) {
        expect(tileSource({ hasTexture: true, hasHeldFrame, unresolved })).toBe("frame");
      }
    }
  });

  /**
   * The fix. Before the key comes back nothing says this tile changed, and for
   * the overwhelming majority it has not — the key is about to be confirmed as
   * the one already on screen. So the held pixels are drawn plainly.
   */
  it("draws an unresolved tile from the held frame, undimmed", () => {
    expect(tileSource({ hasTexture: false, hasHeldFrame: true, unresolved: true })).toBe("held");
  });

  /**
   * And the dim stays where it tells the truth: the key is known, it differs
   * from what is on screen, so the tile really is stale while it fetches.
   */
  it("marks a resolved tile stale while its replacement is on the way", () => {
    expect(tileSource({ hasTexture: false, hasHeldFrame: true, unresolved: false })).toBe(
      "held-stale",
    );
  });

  /** With nothing held there is nothing to fall back to, dimmed or otherwise. */
  it("has nowhere to fall back to without a held frame", () => {
    for (const unresolved of [true, false]) {
      expect(tileSource({ hasTexture: false, hasHeldFrame: false, unresolved })).toBe("frame");
    }
  });
});

describe("knowing a lookup from a fetch", () => {
  /**
   * `unresolved` is what feeds `tileSource` above, under the same name and
   * the same sense, so the call site cannot invert it. What is left to check
   * is that the cache means by it what the renderer thinks it does.
   *
   * The cache is driven without a GL context on purpose — resolution touches
   * only the key map, and a tile whose key is unknown returns before anything
   * would upload a texture.
   */
  it("is unresolved until the frame's keys land, and resolved after", async () => {
    vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));
    const gl = {} as unknown as WebGL2RenderingContext;
    const cache = new TileCache(gl, "ve-tile://");
    let asked = 0;
    cache.resolver = async (_frame, tiles) => {
      asked += 1;
      return tiles.map(() => "content-hash-a");
    };

    const frame = "7/0";
    expect(cache.unresolved(frame, 2, 1, 1)).toBe(true);

    // A draw asks for the tile; the key request is batched into a microtask.
    expect(cache.get(frame, 2, 1, 1)).toBeNull();
    expect(cache.unresolved(frame, 2, 1, 1), "still unknown in the same tick").toBe(true);

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(asked).toBe(1);
    expect(cache.unresolved(frame, 2, 1, 1)).toBe(false);
    // A tile the resolution did not cover is still unknown.
    expect(cache.unresolved(frame, 2, 9, 9)).toBe(true);
    // And another frame is its own question entirely — an edit re-addresses
    // every tile, which is what makes the distinction matter at all.
    expect(cache.unresolved("8/0", 2, 1, 1)).toBe(true);
    cache.dispose();
  });
});

function gpu() {
  return {
    createTexture: vi.fn(() => ({})), deleteTexture: vi.fn(), bindTexture: vi.fn(),
    texImage2D: vi.fn(), texParameteri: vi.fn(),
  } as unknown as WebGL2RenderingContext;
}

describe("preparing playback without animation callbacks", () => {
  const view = [{ z: 0, x: 0, y: 0 }];
  const bytes = new Uint8Array(TILE_BYTES);
  function setup(limit = 768) {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date", "performance"] });
    const gl = gpu();
    const cache = new TileCache(gl, "ve-tile://", limit);
    const resolve = vi.fn(async (frame: string, tiles: readonly { x: number }[]) => tiles.map((tile) => `${frame}:${tile.x}`));
    cache.resolver = resolve;
    const fetch = vi.fn(async () => new Response(bytes));
    vi.stubGlobal("fetch", fetch);
    return { cache, gl, resolve, fetch };
  }

  it("resolves, fetches, and uploads all frames once; a long second lap needs no traffic", async () => {
    const { cache, fetch, resolve, gl } = setup();
    cache.reserve(120, 1);
    const frames = Array.from({ length: 120 }, (_, i) => `7/${i}`);
    cache.prepare(frames, view);
    await vi.runAllTimersAsync();
    expect(cache.prepare(frames, view)).toBe(120);
    expect(fetch).toHaveBeenCalledTimes(120);
    expect(resolve).toHaveBeenCalledTimes(120);
    for (const frame of frames) expect(cache.prefetch(frame, view)).toBe(true);
    await vi.runAllTimersAsync();
    expect(fetch).toHaveBeenCalledTimes(120);
    expect(resolve).toHaveBeenCalledTimes(120);
    expect(gl.texImage2D).toHaveBeenCalledTimes(120);
    cache.dispose();
  });

  it("limits concurrent transfers and keeps bytes awaiting upload within that limit", async () => {
    const { cache, fetch } = setup();
    const pending: Array<(response: Response) => void> = [];
    fetch.mockImplementation(() => new Promise((resolve) => pending.push(resolve)));
    const tiles = Array.from({ length: 100 }, (_, x) => ({ z: 4, x, y: 0 }));
    cache.prepare(["7/0"], tiles);
    await vi.advanceTimersByTimeAsync(0);
    expect(fetch).toHaveBeenCalledTimes(64);
    pending.forEach((resolve) => resolve(new Response(bytes)));
    await vi.runAllTimersAsync();
    expect(fetch).toHaveBeenCalledTimes(100);
    cache.dispose();
  });

  it("protects the displayed and next frame when unrelated tiles exceed capacity", async () => {
    const { cache } = setup(3);
    cache.protect(["7/0", "7/1"], view);
    cache.prepare(["7/0", "7/1", "7/2"], view);
    await vi.runAllTimersAsync();
    expect(cache.residentCount("7/0", view)).toBe(1);
    cache.get("7/3", 0, 0, 0);
    await vi.runAllTimersAsync();
    expect(cache.residentCount("7/0", view)).toBe(1);
    expect(cache.residentCount("7/1", view)).toBe(1);
    expect(cache.stats().ready).toBeLessThanOrEqual(3);
    cache.dispose();
  });

  it("does not fetch an obsolete preparation after its keys resolve", async () => {
    const { cache, fetch } = setup();
    let resolve!: (keys: string[]) => void;
    cache.resolver = () => new Promise((done) => { resolve = done; });
    cache.prepare(["7/0"], view);
    await vi.advanceTimersByTimeAsync(0);
    cache.prepare([], view);
    resolve(["old-key"]);
    await vi.runAllTimersAsync();
    expect(fetch).not.toHaveBeenCalled();
    cache.dispose();
  });

  it("ignores a resolver that completes after disposal", async () => {
    const { cache, fetch } = setup();
    let resolve!: (keys: string[]) => void;
    cache.resolver = () => new Promise((done) => { resolve = done; });
    cache.prepare(["7/0"], view);
    await vi.advanceTimersByTimeAsync(0);
    cache.dispose();
    resolve(["old-key"]);
    await vi.runAllTimersAsync();
    expect(cache.unresolved("7/0", 0, 0, 0)).toBe(true);
    expect(fetch).not.toHaveBeenCalled();
  });

  it("retries failed transfers with a bounded attempt count", async () => {
    const { cache, fetch } = setup();
    fetch.mockImplementation(async () => new Response(null, { status: 503 }));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    for (let attempt = 0; attempt < 5; attempt++) {
      cache.prepare(["7/0"], view);
      await vi.advanceTimersByTimeAsync(10_000);
    }
    expect(fetch).toHaveBeenCalledTimes(3);
    expect(cache.stats().failed).toBe(1);
    cache.dispose();
    warn.mockRestore();
  });
});

describe("sizing the cache to the timeline", () => {
  const cache = () => new TileCache({} as unknown as WebGL2RenderingContext, "ve-tile://");

  /**
   * The whole point of the reservation. Playback is gated on every tile of the
   * next step being resident, and the tiles have to be fetched and uploaded
   * whatever the backend has already rendered — so a loop that does not fit
   * refetches every step on every lap and never reaches the set rate. A loop
   * that fits pays once.
   */
  it("holds a whole timeline once it is reserved for one", () => {
    const tiles = cache();
    expect(tiles.capacity, "the floor, before anything is known").toBe(BASE_CAPACITY);

    tiles.reserve(24, 128);
    expect(tiles.capacity).toBeGreaterThanOrEqual(24 * 128);
  });

  /** A short timeline must not shrink the cache below what a viewport needs. */
  it("never drops below the floor", () => {
    const tiles = cache();
    tiles.reserve(2, 8);
    expect(tiles.capacity).toBe(BASE_CAPACITY);
  });

  /**
   * The ceiling is what keeps a long timeline at a big viewport from asking
   * for more texture memory than a GPU has: 240 steps of a 170-tile viewport
   * is 40,800 tiles, or ten gigabytes.
   */
  it("stops at the memory ceiling rather than asking for the impossible", () => {
    const tiles = cache();
    tiles.reserve(240, 170);
    expect(tiles.capacity).toBe(MAX_CAPACITY);
    expect(MAX_CAPACITY * TILE_BYTES).toBeLessThanOrEqual(MAX_CACHE_BYTES);
  });

  /** Re-reserving for a smaller need releases the memory again. */
  it("comes back down when the timeline or the viewport shrinks", () => {
    const tiles = cache();
    tiles.reserve(96, 128);
    const large = tiles.capacity;
    tiles.reserve(12, 32);
    expect(tiles.capacity).toBeLessThan(large);
    expect(tiles.capacity).toBeGreaterThanOrEqual(BASE_CAPACITY);
  });
});

describe("how far a live edit reaches", () => {
  /** An unscoped tool is aimed at no layer, so it applies to the whole stack. */
  it("applies everywhere when nothing asked for a scope", () => {
    expect(editScope(false, false)).toBe("everywhere");
    expect(editScope(false, true)).toBe("everywhere");
  });

  it("applies to the layer's own pixels once the scope is resident", () => {
    expect(editScope(true, true)).toBe("layer");
  });

  /**
   * The regression (M72). This fell back to "everywhere", which meant an
   * intensity aimed at a current layer intensified the wind beneath it — for
   * the whole stroke, since nothing fetched the scope once the button was
   * down. An asked-for scope must never be dropped.
   */
  it("applies nowhere while an asked-for scope has not arrived", () => {
    expect(editScope(true, false)).toBe("nowhere");
    expect(editScope(true, false)).not.toBe("everywhere");
  });
});
