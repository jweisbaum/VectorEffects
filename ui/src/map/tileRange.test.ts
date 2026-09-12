import { describe, expect, it } from "vitest";

import { SPEED_MAX, tileSpeedRange } from "./tileRange";

/** One texel's word, as `ve_render::tile::encode_texel` packs it. */
function word(speed: number, coverage: number, kind: "wind" | "current", azimuth = 0x123): number {
  return (
    ((speed & SPEED_MAX) |
      ((azimuth & 0xfff) << 14) |
      ((coverage & 0x1f) << 26) |
      ((kind === "wind" ? 1 : 0) << 31)) >>>
    0
  );
}

/** A tile of texels, little-endian. */
function tile(words: number[]): Uint8Array {
  const bytes = new Uint8Array(words.length * 4);
  words.forEach((w, i) => {
    bytes[i * 4] = w & 0xff;
    bytes[i * 4 + 1] = (w >>> 8) & 0xff;
    bytes[i * 4 + 2] = (w >>> 16) & 0xff;
    bytes[i * 4 + 3] = (w >>> 24) & 0xff;
  });
  return bytes;
}

describe("tileSpeedRange", () => {
  it("reads the low and high speed of each kind out of the words", () => {
    const bytes = tile([
      word(0x0102, 31, "wind"),
      word(0x0a0b, 31, "wind"),
      word(0x0304, 31, "wind"),
      word(7, 31, "current"),
      word(90, 31, "current"),
    ]);
    expect(tileSpeedRange(bytes)).toEqual({ wind: [0x0102, 0x0a0b], current: [7, 90] });
  });

  it("leaves unwritten cells out and counts a written calm", () => {
    expect(tileSpeedRange(tile([word(0, 0, "wind"), word(500, 31, "wind"), word(0, 31, "wind")]))).toEqual({
      wind: [0, 500],
      current: null,
    });
  });

  it("is null for a kind the tile holds none of", () => {
    expect(tileSpeedRange(tile([word(0, 0, "wind"), word(0, 0, "current")]))).toEqual({
      wind: null,
      current: null,
    });
    expect(tileSpeedRange(new Uint8Array(0))).toEqual({ wind: null, current: null });
  });

  it("ignores the direction bits", () => {
    expect(tileSpeedRange(tile([word(1000, 3, "current", 0xfff), word(1000, 3, "current", 0)]))).toEqual({
      wind: null,
      current: [1000, 1000],
    });
  });

  it("reads the top speed the word can hold", () => {
    expect(tileSpeedRange(tile([word(SPEED_MAX, 31, "wind")]))).toEqual({
      wind: [SPEED_MAX, SPEED_MAX],
      current: null,
    });
  });
});

describe("the fade at an edge", () => {
  /**
   * The report (M81): the low end of the scale never moved. A cell's vector is
   * premultiplied by its coverage, so the rim of every feathered object ramps
   * from the object's speed down to nothing — and a feather is the default, so
   * whatever was on screen there was always a cell most of the way down one.
   * The bottom of the ramp was pinned near zero and only the top responded to
   * the view.
   */
  it("is left out, so the scale spans the field and not the feather", () => {
    const bytes = tile([
      word(40, 2, "wind"), // the fade at a rim: mostly uncovered
      word(80, 8, "wind"),
      word(900, 31, "wind"), // the field itself
      word(1200, 20, "wind"),
    ]);
    expect(tileSpeedRange(bytes).wind).toEqual([900, 1200]);
  });

  /** Half covered is field; a hair under is the fade. */
  it("splits at half coverage", () => {
    expect(tileSpeedRange(tile([word(500, 16, "wind"), word(900, 31, "wind")])).wind).toEqual([
      500, 900,
    ]);
    expect(tileSpeedRange(tile([word(500, 15, "wind"), word(900, 31, "wind")])).wind).toEqual([
      900, 900,
    ]);
  });

  /**
   * A kind with nothing but fade still gets a scale. A thin rim and no
   * interior is a real thing to look at, and reporting no range at all would
   * drop it back to the project's fixed scale.
   */
  it("falls back to the faded cells when a kind has no solid one", () => {
    const bytes = tile([word(40, 2, "current"), word(80, 8, "current")]);
    expect(tileSpeedRange(bytes).current).toEqual([40, 80]);
  });

  /** And the two kinds decide separately. */
  it("decides per kind", () => {
    const bytes = tile([
      word(40, 2, "current"),
      word(900, 31, "wind"),
      word(1200, 31, "wind"),
    ]);
    expect(tileSpeedRange(bytes)).toEqual({ wind: [900, 1200], current: [40, 40] });
  });
});
