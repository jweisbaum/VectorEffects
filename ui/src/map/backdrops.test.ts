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
      "ve-tile://localhost/backdrop/sst/4242/7/2/5/1.bin",
      "ve-tile://localhost/backdrop/sst/4242/7/2/6/1.bin",
    ]);
    // Drawn over the basemap, never in place of it: land is clear in the tile.
    expect(draws[0]?.replacesBase).toBe(false);
    // Temperatures for the map to colour, not a picture.
    expect(draws[0]?.temperature).toBe(true);
  });

  /**
   * A tile that arrives brings its coldest and warmest water with it, which
   * is what the auto scale spans. Two pixels of water, 12 °C and 20 °C, and
   * the rest land.
   */
  it("hands the next draw its tile's temperatures", async () => {
    const bytes = new Uint8Array(256 * 256 * 4);
    const put = (at: number, celsius: number) => {
      const code = Math.round(((celsius + 5) / 50) * 65535);
      bytes.set([code & 0xff, code >> 8, 0, 255], at * 4);
    };
    put(0, 12);
    put(1000, 20);
    vi.stubGlobal("fetch", () => Promise.resolve(new Response(bytes, { status: 200 })));
    const gl = {
      TEXTURE_2D: 0, createTexture: () => ({}), bindTexture: () => {}, texParameteri: () => {},
      texImage2D: () => {},
    } as unknown as WebGL2RenderingContext;
    const backdrops = new BackdropCache(gl, "ve-tile://localhost/");
    const arrived = new Promise<void>((resolve) => { backdrops.onChange = resolve; });
    backdrops.draws({ sst: [{ layer: 7, token: 4242 }] }, TILES.slice(0, 1));
    await arrived;
    const [draw] = backdrops.draws({ sst: [{ layer: 7, token: 4242 }] }, TILES.slice(0, 1));
    const range = draw?.textures[0]?.range;
    expect(range?.[0]).toBeCloseTo(12, 3);
    expect(range?.[1]).toBeCloseTo(20, 3);
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
