/**
 * The history import's range rules (spec 4.10, M38).
 *
 * Every one of these is about *time*, and the whole reason they are in a
 * module of their own is that a time rule checked only by clicking through a
 * dialog is a time rule nothing checks. The reference is arithmetic done by
 * hand, not a second copy of the code's formula.
 */
import { describe, expect, it } from "vitest";

import {
  ARCHIVES,
  MAX_HOURS,
  defaultRange,
  formatUtcHour,
  parseUtcHour,
  rangeState,
} from "./historyRange";

const HOUR = 3600;
const DAY = 24 * HOUR;

describe("reading a datetime-local value", () => {
  /**
   * The control has no zone, and the archives are UTC. Reading the value as
   * local time would shift every fetched hour by the reader's own offset —
   * silently, and by a different amount for every user.
   */
  it("reads the value as UTC and never as local time", () => {
    // 1970-01-02T00:00Z is exactly one day after the epoch.
    expect(parseUtcHour("1970-01-02T00:00")).toBe(DAY);
    // 2000-01-01T00:00Z: 30 years, of which 1972..1996 gives 7 leap days.
    expect(parseUtcHour("2000-01-01T00:00")).toBe((30 * 365 + 7) * DAY);
    expect(parseUtcHour("1970-01-01T13:00")).toBe(13 * HOUR);
  });

  it("keeps a seconds field out of the answer", () => {
    expect(parseUtcHour("1970-01-01T05:00:00")).toBe(5 * HOUR);
  });

  /** A day the month does not have would otherwise roll into the next one. */
  it("refuses a date that does not exist", () => {
    expect(parseUtcHour("2001-02-29T00:00")).toBeNull();
    expect(parseUtcHour("2001-13-01T00:00")).toBeNull();
    expect(parseUtcHour("2001-04-31T00:00")).toBeNull();
    expect(parseUtcHour("2000-02-29T00:00")).not.toBeNull();
  });

  it("refuses anything that is not a date and an hour", () => {
    expect(parseUtcHour("")).toBeNull();
    expect(parseUtcHour("2020-01-01")).toBeNull();
    expect(parseUtcHour("tomorrow")).toBeNull();
  });

  it("round trips through the control's format", () => {
    for (const value of ["1970-01-01T00:00", "2026-09-07T13:00", "1999-12-31T23:00"]) {
      const seconds = parseUtcHour(value);
      expect(seconds, value).not.toBeNull();
      expect(formatUtcHour(seconds as number)).toBe(value);
    }
  });

  /** A time part-way through an hour is shown as the hour it is in. */
  it("writes back on the hour", () => {
    expect(formatUtcHour(59 * 60)).toBe("1970-01-01T00:00");
  });
});

describe("judging a range", () => {
  const both = ARCHIVES.map((a) => a.id);
  const state = (start: string, end: string, archives = both) =>
    rangeState(start, end, archives);

  /** Both ends count, so one hour of data is one hour and not none. */
  it("counts both ends", () => {
    expect(state("2020-01-01T00:00", "2020-01-01T00:00").hours).toBe(1);
    expect(state("2020-01-01T00:00", "2020-01-01T01:00").hours).toBe(2);
    expect(state("2020-01-01T00:00", "2020-01-02T00:00").hours).toBe(25);
  });

  it("accepts a range it can fetch", () => {
    expect(state("2020-01-01T00:00", "2020-01-01T23:00").problem).toBeNull();
  });

  /**
   * The cap is the backend's, restated so the wait is declined before it
   * starts. The boundary is what matters: exactly the cap is allowed and one
   * more is not.
   */
  it("allows exactly the cap and refuses one hour more", () => {
    const start = parseUtcHour("2020-01-01T00:00") as number;
    const at = (hours: number) => formatUtcHour(start + hours * HOUR);
    expect(state(at(0), at(MAX_HOURS - 1)).problem).toBeNull();
    const over = state(at(0), at(MAX_HOURS));
    expect(over.hours).toBe(MAX_HOURS + 1);
    expect(over.problem).toContain(String(MAX_HOURS + 1));
  });

  it("refuses a backwards range", () => {
    expect(state("2020-01-02T00:00", "2020-01-01T00:00").problem).toContain("before the start");
  });

  it("refuses an unreadable date", () => {
    expect(state("", "2020-01-01T00:00").problem).not.toBeNull();
  });

  /** An import with no archive would fetch nothing and add no layer. */
  it("refuses when no archive is chosen", () => {
    expect(state("2020-01-01T00:00", "2020-01-01T01:00", []).problem).toContain("archive");
  });
});

describe("the range the dialog opens on", () => {
  /**
   * Both archives trail real time by days at best, so a range ending now is
   * a range neither has. The default has to be one that works, or the first
   * thing a new user sees is an error from the archive.
   */
  it("is a day, ending a week back, and is itself valid", () => {
    const now = parseUtcHour("2026-09-07T13:00") as number;
    // Half past, to prove the default lands on the hour rather than carrying
    // whatever minute the dialog happened to open at.
    const { start, end } = defaultRange(now + 30 * 60);
    expect(end).toBe(formatUtcHour(now - 7 * DAY));
    const judged = rangeState(start, end, ARCHIVES.map((a) => a.id));
    expect(judged.problem).toBeNull();
    expect(judged.hours).toBe(24);
  });
});
