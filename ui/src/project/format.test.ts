import { describe, expect, it } from "vitest";

import { displayDirection, formatBytes, gridSize, knotsFromMps, mpsFromKnots } from "./format";

describe("gridSize", () => {
  /** Checked against the table in spec.md 4.2. */
  it("matches the specified grid dimensions", () => {
    expect(gridSize(1)).toEqual({ ni: 360, nj: 181 });
    expect(gridSize(0.5)).toEqual({ ni: 720, nj: 361 });
    expect(gridSize(0.25)).toEqual({ ni: 1440, nj: 721 });
    expect(gridSize(0.1)).toEqual({ ni: 3600, nj: 1801 });
  });
});

describe("speed conversion", () => {
  /** Must match `ve_core::units`, which is the authority. */
  it("converts between the stored and displayed units", () => {
    expect(knotsFromMps(1)).toBeCloseTo(1.943844, 5);
    expect(mpsFromKnots(34)).toBeCloseTo(17.49, 2);
    expect(knotsFromMps(0)).toBe(0);
  });

  it("round trips", () => {
    for (const mps of [0, 0.5, 12.5, 33.3, 120]) {
      expect(mpsFromKnots(knotsFromMps(mps))).toBeCloseTo(mps, 9);
    }
  });
});

describe("displayDirection", () => {
  /**
   * A northerly wind blows *from* the north, so it points south: stored
   * azimuth 180, displayed as 0 under the meteorological convention.
   */
  it("applies the meteorological convention for wind", () => {
    expect(displayDirection("from", 180)).toBeCloseTo(0, 9);
    expect(displayDirection("from", 0)).toBeCloseTo(180, 9);
    expect(displayDirection("from", 270)).toBeCloseTo(90, 9);
  });

  it("leaves the oceanographic convention alone", () => {
    expect(displayDirection("toward", 180)).toBeCloseTo(180, 9);
    expect(displayDirection("toward", 45)).toBeCloseTo(45, 9);
  });
});

describe("formatBytes", () => {
  it("scales to a readable unit", () => {
    expect(formatBytes(500 * 1024)).toBe("500 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5 MB");
    expect(formatBytes(3 * 1024 * 1024 * 1024)).toBe("3.0 GB");
  });
});
