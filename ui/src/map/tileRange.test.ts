import { describe, expect, it } from "vitest";

import { tileSpeedRange } from "./tileRange";

/** A tile of `speeds`, each a 16-bit fraction, with arbitrary directions. */
function tile(speeds: number[]): Uint8Array {
  const bytes = new Uint8Array(speeds.length * 4);
  speeds.forEach((speed, i) => {
    bytes[i * 4] = speed & 0xff;
    bytes[i * 4 + 1] = speed >> 8;
    bytes[i * 4 + 2] = 0x34;
    bytes[i * 4 + 3] = 0x12;
  });
  return bytes;
}

describe("tileSpeedRange", () => {
  it("reads the low and high speed out of the byte pairs, little-endian", () => {
    expect(tileSpeedRange(tile([0x0102, 0x0a0b, 0x0304]))).toEqual([0x0102, 0x0a0b]);
  });

  it("leaves unpainted cells out of the minimum", () => {
    expect(tileSpeedRange(tile([0, 500, 0, 700]))).toEqual([500, 700]);
  });

  it("is null for a tile with no field in it", () => {
    expect(tileSpeedRange(tile([0, 0, 0]))).toBeNull();
    expect(tileSpeedRange(new Uint8Array(0))).toBeNull();
  });

  it("ignores the direction bytes", () => {
    const bytes = tile([1000]);
    bytes[2] = 0xff;
    bytes[3] = 0xff;
    expect(tileSpeedRange(bytes)).toEqual([1000, 1000]);
  });
});
