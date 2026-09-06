import { describe, expect, it } from "vitest";

import { nearestHour, startDraftFrom, unixOf } from "./Timeline";

describe("the start time", () => {
  it("rounds now to the nearest hour, UTC", () => {
    expect(nearestHour(new Date(Date.UTC(2026, 8, 6, 8, 29))).toISOString()).toBe(
      "2026-09-06T08:00:00.000Z",
    );
    expect(nearestHour(new Date(Date.UTC(2026, 8, 6, 8, 30))).toISOString()).toBe(
      "2026-09-06T09:00:00.000Z",
    );
    expect(nearestHour(new Date(Date.UTC(2026, 11, 31, 23, 45))).toISOString()).toBe(
      "2027-01-01T00:00:00.000Z",
    );
  });

  it("round-trips a start time through the draft", () => {
    const unix = Date.UTC(2026, 8, 6, 12) / 1000;
    expect(startDraftFrom(unix)).toEqual({ year: 2026, month: 9, day: 6, hour: 12 });
    expect(unixOf({ year: 2026, month: 9, day: 6, hour: 12 })).toBe(unix);
  });

  it("refuses a date that does not exist", () => {
    expect(unixOf({ year: 2026, month: 2, day: 30, hour: 0 })).toBeNull();
    expect(unixOf({ year: 2024, month: 2, day: 29, hour: 0 })).not.toBeNull();
  });
});
