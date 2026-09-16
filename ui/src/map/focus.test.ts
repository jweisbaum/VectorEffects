import { describe, expect, it } from "vitest";

import { clampCamera } from "./camera";

describe("focus camera", () => {
  it("keeps the projection and clamps latitude", () => {
    const view = { width: 800, height: 600 };
    const before = { centerLon: 0, centerLat: 20, pxPerDeg: 3, projection: "mercator" as const };
    const after = clampCamera({ ...before, centerLon: -70.5, centerLat: 95, pxPerDeg: 12 }, view);
    expect(after.projection).toBe("mercator");
    expect(after.centerLon).toBeCloseTo(-70.5);
    expect(after.centerLat).toBeLessThanOrEqual(90);
    expect(after.pxPerDeg).toBe(12);
  });
});
