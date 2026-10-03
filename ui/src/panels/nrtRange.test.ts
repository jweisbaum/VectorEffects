/**
 * The near-real-time import's period and cost (spec 4.10, M89).
 *
 * The reference is arithmetic done by hand — the same cases the backend's own
 * tests assert for `ve_app::nrt::period` and `wanted_times` — not a second
 * copy of the code's formula. The dialog and the import must count alike, or
 * the dialog promises a number of downloads the import does not make.
 */
import { describe, expect, it } from "vitest";

import {
  MAX_STEPS,
  NRT_PRODUCTS,
  clampDays,
  costOf,
  maxDays,
  periodOf,
  wantedTimes,
} from "./nrtRange";

const HOUR = 3600;
const DAY = 24 * HOUR;

/** 2026-10-02T15:40Z. */
const NOW = 1_790_955_600;
/** 2026-10-02T00:00Z, the midnight NOW's day began on. */
const MIDNIGHT = 1_790_899_200;

describe("the period", () => {
  it("is this instant, so the cases below are about the day they say", () => {
    expect(new Date(NOW * 1000).toISOString()).toBe("2026-10-02T15:40:00.000Z");
    expect(new Date(MIDNIGHT * 1000).toISOString()).toBe("2026-10-02T00:00:00.000Z");
  });

  /**
   * Three days back from 2 October is 29 September at midnight; the end is
   * 15:00, the hour 15:40 is in. That is 3 × 24 + 15 = 87 hours, and both
   * ends count.
   */
  it("starts on a midnight and ends on the current hour", () => {
    expect(periodOf(NOW, 3, 1)).toEqual({
      startUnixS: 1_790_640_000,
      endUnixS: 1_790_953_200,
      steps: 88,
    });
    expect(1_790_640_000).toBe(MIDNIGHT - 3 * DAY);
    expect(1_790_953_200).toBe(MIDNIGHT + 15 * HOUR);
  });

  /** A coarser timeline needs fewer steps for the same span. */
  it("counts steps of the project's own length", () => {
    // 10 × 24 + 15 = 255 hours, 256 steps: more than a timeline holds.
    expect(periodOf(NOW, 10, 1).steps).toBe(256);
    // 9 × 24 + 15 = 231 hours.
    expect(periodOf(NOW, 9, 1).steps).toBe(232);
    // 28 × 24 + 15 = 687 hours; 687 / 3 = 229 whole strides.
    expect(periodOf(NOW, 28, 3).steps).toBe(230);
  });

  /**
   * "Today" is the UTC day. A second either side of midnight is a whole day
   * apart in where the period starts, whatever the reader's own clock says.
   */
  it("takes the day from UTC, on both sides of midnight", () => {
    expect(periodOf(MIDNIGHT - 1, 1, 1).startUnixS).toBe(1_790_726_400);
    expect(periodOf(MIDNIGHT + 1, 1, 1).startUnixS).toBe(1_790_812_800);
    expect(1_790_726_400).toBe(MIDNIGHT - 2 * DAY);
    expect(1_790_812_800).toBe(MIDNIGHT - DAY);
  });
});

describe("the most days a timeline can take", () => {
  it("is the last number of days that fits in the timeline", () => {
    expect(MAX_STEPS).toBe(240);
    // Nine days is 232 hourly steps and ten is 256.
    expect(maxDays(NOW, 1)).toBe(9);
    // Three-hourly: 29 days is 711 hours, 238 steps; 30 days is 735, 246.
    expect(maxDays(NOW, 3)).toBe(29);
    // Daily: a day more is a step more, and day 0 is a step too.
    expect(maxDays(NOW, 24)).toBe(239);
  });

  /** The hour matters: at midnight a day is 24 hours of period, not 39. */
  it("depends on the hour of the day", () => {
    // At 00:00, nine days is 217 steps and ten is 241 — one too many.
    expect(maxDays(MIDNIGHT, 1)).toBe(9);
    // At 23:59, nine days is 240 steps exactly.
    expect(periodOf(MIDNIGHT + DAY - 1, 9, 1).steps).toBe(240);
    expect(maxDays(MIDNIGHT + DAY - 1, 1)).toBe(9);
  });

  it("is never less than one", () => {
    expect(maxDays(NOW, 1)).toBeGreaterThanOrEqual(1);
    expect(maxDays(NOW, 6)).toBeGreaterThanOrEqual(1);
  });

  it("holds a typed number of days to what can be asked for", () => {
    expect(clampDays(3, 9)).toBe(3);
    expect(clampDays(0, 9)).toBe(1);
    expect(clampDays(40, 9)).toBe(9);
    expect(clampDays(2.4, 9)).toBe(2);
    expect(clampDays(Number.NaN, 9)).toBe(1);
  });
});

describe("the times fetched for one product", () => {
  const p = periodOf(NOW, 3, 1);

  /**
   * A daily product is fetched at its midnights however fine the timeline:
   * 29 and 30 September, 1 and 2 October.
   */
  it("walks a daily product a day at a time", () => {
    expect(wantedTimes(p, 1, 88, 24)).toBe(4);
    // A six-hourly timeline of 15 steps reaches 84 h: the same four midnights.
    expect(wantedTimes(p, 6, 15, 24)).toBe(4);
  });

  /** An hourly product on a three-hourly timeline is every third hour. */
  it("walks an hourly product at the timeline's step", () => {
    // 0, 3, … 87 h: thirty times.
    expect(wantedTimes(p, 3, 30, 1)).toBe(30);
    expect(wantedTimes(p, 1, 88, 1)).toBe(88);
  });

  /** A step past the timeline's last has nowhere to be shown. */
  it("stops at the end of the timeline", () => {
    expect(wantedTimes(p, 3, 10, 1)).toBe(10);
    // Twenty-four hourly steps end at 23 h: the first midnight only.
    expect(wantedTimes(p, 1, 24, 24)).toBe(1);
    // One step more reaches 24 h, which is the second midnight.
    expect(wantedTimes(p, 1, 25, 24)).toBe(2);
  });
});

describe("the cost", () => {
  const p = periodOf(NOW, 3, 1);
  const all = NRT_PRODUCTS.map((product) => product.id);

  it("lists the products in the order the import reads them", () => {
    expect(all).toEqual([
      "multiobs",
      "duacs",
      "wind-l4",
      "ascat",
      "ccmp",
      "seawinds",
      "oisst",
      "geopolar",
      "ostia",
    ]);
  });

  /**
   * 88 hourly currents, 4 daily currents, 88 hourly winds, 4 daily
   * scatterometer winds, 15 six-hourly CCMP winds (0 h to 84 h of an
   * 87-hour period), 15 six-hourly Blended Seawinds in 4 day files, and 4
   * days of each of the three daily temperatures: 88 + 4 + 88 + 4 + 15 + 4
   * + 3 × 4 = 215 downloads. 88 × 2.8 + 4 × 2.8 + 88 × 4.2 + 4 × 4.2 + 15 ×
   * 3.7 + 15 × 4.2 + 12 × 1.9 = 246.4 + 11.2 + 369.6 + 16.8 + 55.5 + 63 +
   * 22.8 = 785.3, which rounds to 785 MB.
   */
  it("sums the downloads and the size over the products chosen", () => {
    expect(costOf(all, p, 1, 88)).toEqual({ downloads: 215, megabytes: 785 });
  });

  /**
   * A timeline left at 24 steps shows one day: 24 + 1 + 24 + 1 + 4 + 1 + 3
   * × 1 = 58 downloads — Blended Seawinds' four times are one file — and
   * 67.2 + 2.8 + 100.8 + 4.2 + 14.8 + 16.8 + 3 × 1.9 = 212.3, so 212 MB.
   */
  it("fetches only what the timeline can show", () => {
    expect(costOf(all, p, 1, 24)).toEqual({ downloads: 58, megabytes: 212 });
  });

  /** A daily timeline takes one time from each day: five days, five files. */
  it("counts a day file per day on a timeline coarser than the product", () => {
    expect(costOf(["seawinds"], periodOf(NOW, 5, 24), 24, 5)).toEqual({
      downloads: 5,
      megabytes: 21,
    });
  });

  /** Five six-hourly times reach into a second day file: two downloads. */
  it("counts a day file once for the times it holds", () => {
    expect(costOf(["seawinds"], p, 6, 5)).toEqual({ downloads: 2, megabytes: 21 });
  });

  it("counts only the products chosen", () => {
    // Four daily times at 2.8 MB is 11.2 MB.
    expect(costOf(["duacs"], p, 1, 88)).toEqual({ downloads: 4, megabytes: 11 });
    // Eighty-eight hourly winds at 4.2 MB is 369.6 MB.
    expect(costOf(["wind-l4"], p, 1, 88)).toEqual({ downloads: 88, megabytes: 370 });
    // A temperature is daily whatever the timeline: four days at 1.9 MB is
    // 7.6 MB, and on a 24-step timeline one day, 1.9 MB.
    expect(costOf(["ostia"], p, 1, 88)).toEqual({ downloads: 4, megabytes: 8 });
    expect(costOf(["oisst"], p, 1, 24)).toEqual({ downloads: 1, megabytes: 2 });
    expect(costOf([], p, 1, 88)).toEqual({ downloads: 0, megabytes: 0 });
  });
});
