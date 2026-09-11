/**
 * Where a tile on screen comes from, and when it is marked stale (M70).
 *
 * The regression this pins: an edit re-addresses every tile on the map, since
 * the address carries the revision, so for one round trip after every stroke
 * no tile of the new frame has a key yet. Dimming on that flashed the whole
 * map — thousands of kilometres from the edit — and then resolved back to full
 * brightness in rectangular batches as the keys landed.
 */
import { describe, expect, it } from "vitest";

import { TileCache, tileSource } from "./tiles";

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
  });
});
