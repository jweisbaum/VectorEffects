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
  MAX_FETCHED_STEPS,
  defaultRange,
  formatUtcHour,
  parseUtcHour,
  rangeState,
  stepsInRange,
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

describe("counting the steps a range covers", () => {
  const start = parseUtcHour("2020-01-01T00:00") as number;
  const at = (hours: number) => start + hours * HOUR;

  /**
   * The point of the change: a three-hourly project downloads every third
   * hour. A step shows an imported message only at its own forecast hour, so
   * the two hours in between could never be drawn and are never fetched.
   */
  it("strides by the project's step", () => {
    expect(stepsInRange(start, at(24), 3, 240), "a day, three-hourly").toBe(9);
    expect(stepsInRange(start, at(24), 1, 240), "the same day, hourly").toBe(25);
    expect(stepsInRange(start, at(24), 6, 240), "six-hourly").toBe(5);
    expect(stepsInRange(start, at(24), 24, 240), "daily").toBe(2);
  });

  /** An hour past the last step has no step to land on. */
  it("stops at the end of the timeline", () => {
    expect(stepsInRange(start, at(1000), 3, 8)).toBe(8);
  });

  /** Both ends count, so one step of data is one step and not none. */
  it("counts both ends", () => {
    expect(stepsInRange(start, start, 1, 240)).toBe(1);
    expect(stepsInRange(start, at(1), 1, 240)).toBe(2);
  });

  /** An end short of the next step does not reach it. */
  it("does not round up to a step the range stops before", () => {
    expect(stepsInRange(start, at(3) - 1, 3, 240)).toBe(1);
    expect(stepsInRange(start, at(3), 3, 240)).toBe(2);
  });

  it("is zero for a backwards range", () => {
    expect(stepsInRange(at(5), start, 1, 240)).toBe(0);
  });
});

describe("judging a range", () => {
  const both = ARCHIVES.map((a) => a.id);
  const state = (start: string, end: string, stepHours = 1, archives = both) =>
    rangeState(start, end, archives, stepHours, 240);

  it("accepts a range it can fetch", () => {
    expect(state("2020-01-01T00:00", "2020-01-01T23:00").problem).toBeNull();
    expect(state("2020-01-01T00:00", "2020-01-01T23:00").steps).toBe(24);
  });

  /**
   * The project's own step count is the real ceiling, so a range longer than
   * the timeline is clamped rather than refused: the extra hours have no step
   * to land on, which is a reason not to fetch them and not a reason to
   * decline the import.
   */
  it("clamps a range longer than the timeline instead of refusing it", () => {
    const judged = rangeState(
      "2020-01-01T00:00",
      "2021-01-01T00:00",
      both,
      3,
      24,
    );
    expect(judged.steps).toBe(24);
    expect(judged.problem).toBeNull();
  });

  /**
   * The cap counts downloads rather than the span named. A project cannot
   * hold more steps than the cap, so this is a guard the app should never
   * reach — but if it ever did, the refusal has to say what it refused.
   */
  it("refuses more steps than one import fetches", () => {
    const over = rangeState(
      "2020-01-01T00:00",
      "2021-01-01T00:00",
      both,
      1,
      MAX_FETCHED_STEPS + 5,
    );
    expect(over.steps).toBe(MAX_FETCHED_STEPS + 5);
    expect(over.problem).toContain(String(MAX_FETCHED_STEPS + 5));
  });

  /**
   * The same wait reaches much further back on a coarser project, because the
   * count is of downloads and not of hours.
   */
  it("costs a coarser project far fewer downloads for the same span", () => {
    const start = parseUtcHour("2020-01-01T00:00") as number;
    const at = (hours: number) => formatUtcHour(start + hours * HOUR);
    // Ten days. Hourly it fills a 240-step timeline exactly.
    const [from, to] = [at(0), at(240)];
    expect(state(from, to, 1, both).steps).toBe(240);
    expect(state(from, to, 6, both).steps).toBe(41);
    expect(state(from, to, 24, both).steps).toBe(11);
  });

  it("refuses a backwards range", () => {
    expect(state("2020-01-02T00:00", "2020-01-01T00:00").problem).toContain("before the start");
  });

  it("refuses an unreadable date", () => {
    expect(state("", "2020-01-01T00:00").problem).not.toBeNull();
  });

  /** An import with no archive would fetch nothing and add no layer. */
  it("refuses when no archive is chosen", () => {
    expect(state("2020-01-01T00:00", "2020-01-01T01:00", 1, []).problem).toContain("archive");
  });

  /** A range that lands on no step of the project fetches nothing. */
  it("refuses a range shorter than one step", () => {
    expect(rangeState("2020-01-02T00:00", "2020-01-01T00:00", both, 3, 24).steps).toBe(0);
  });
});

describe("the range the dialog opens on", () => {
  /**
   * Both archives trail real time, and ERA5's own attributes overstate what
   * it holds by about two days: measured on 2026-09-07 it advertised hours
   * through 2026-09-01 and had written only to 2026-08-30. A week back landed
   * in that gap and failed on the first hour, so the default sits a fortnight
   * back, clear of both the lag and the overstatement.
   */
  it("ends far enough back to clear the archives' lag", () => {
    const now = parseUtcHour("2026-09-07T13:00") as number;
    // Half past, to prove the default lands on the hour rather than carrying
    // whatever minute the dialog happened to open at.
    const { end } = defaultRange(now + 30 * 60);
    const endAt = parseUtcHour(end) as number;
    expect(now - endAt, "at least ten days back").toBeGreaterThanOrEqual(10 * DAY);
    expect(endAt % HOUR, "on the hour").toBe(0);
  });

  it("is a whole day, and is itself a range the dialog accepts", () => {
    const now = parseUtcHour("2026-09-07T13:00") as number;
    const { start, end } = defaultRange(now);
    expect((parseUtcHour(end) as number) - (parseUtcHour(start) as number)).toBe(23 * HOUR);
    const judged = rangeState(start, end, ARCHIVES.map((a) => a.id), 1, 240);
    expect(judged.problem).toBeNull();
    expect(judged.steps).toBe(24);
  });

  /** On a coarser project the same default is proportionally fewer reads. */
  it("costs a three-hourly project eight steps", () => {
    const { start, end } = defaultRange(parseUtcHour("2026-09-07T13:00") as number);
    expect(rangeState(start, end, ARCHIVES.map((a) => a.id), 3, 240).steps).toBe(8);
  });
});
