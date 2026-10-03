/**
 * What the backdrops ask the backend for (spec.md 4.10, 4.11).
 *
 * The addresses are the contract with `protocol::serve_backdrop`: a key the
 * backend does not recognise is a layer that never draws, and nothing on
 * screen says why.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { BackdropCache, layerBackdrops } from "./backdrops";
import type { VisibleTile } from "./camera";

const TILES: VisibleTile[] = [
  { z: 2, x: 5, y: 1, lonOffset: 0 },
  { z: 2, x: 6, y: 1, lonOffset: 0 },
];

let requested: string[];

beforeEach(() => {
  requested = [];
  // Every fetch answers 204, "nothing here": the test is about which
  // addresses are asked for, not about decoding a picture.
  vi.stubGlobal("fetch", (address: string) => {
    requested.push(address);
    return Promise.resolve(new Response(null, { status: 204 }));
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function cache(): BackdropCache {
  // Nothing is uploaded when every answer is 204, so no method is reached.
  return new BackdropCache({} as WebGL2RenderingContext, "ve-tile://localhost/");
}

describe("a sea-surface temperature layer", () => {
  it("asks for its day's tiles under its own layer", () => {
    const draws = cache().draws({ sst: [{ layer: 7, token: 4242 }] }, TILES);
    expect(draws.map((draw) => draw.key)).toEqual(["backdrop/sst/4242/7"]);
    expect(requested).toEqual([
      "ve-tile://localhost/backdrop/sst/4242/7/2/5/1.png",
      "ve-tile://localhost/backdrop/sst/4242/7/2/6/1.png",
    ]);
    // Drawn over the basemap, never in place of it: land is clear in the tile.
    expect(draws[0]?.replacesBase).toBe(false);
  });

  it("asks for nothing when no layer has a day at this step", () => {
    expect(cache().draws({ sst: [] }, TILES)).toEqual([]);
    expect(requested).toEqual([]);
  });
});

describe("the layers' backdrops", () => {
  /** A GIS layer over a temperature layer is drawn after it, and the reverse. */
  it("follow the stack across both kinds", () => {
    expect(
      layerBackdrops({
        gis: [{ layer: 1, token: 9, stack: 2 }],
        sst: [
          { layer: 2, token: 30, stack: 0 },
          { layer: 3, token: 31, stack: 3 },
        ],
      }),
    ).toEqual([
      { kind: "sst", layer: 2, token: 30 },
      { kind: "gis", layer: 1, token: 9 },
      { kind: "sst", layer: 3, token: 31 },
    ]);
  });

  it("keep the order given when no position is known", () => {
    expect(
      layerBackdrops({
        gis: [{ layer: 1, token: 9 }],
        sst: [{ layer: 2, token: 30 }],
      }).map((entry) => entry.kind),
    ).toEqual(["gis", "sst"]);
  });
});
