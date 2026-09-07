/**
 * The date range a history import asks for (spec 4.10, M38).
 *
 * Kept apart from the dialog because every rule here is a rule about
 * *times*, and a time rule that is only checked by clicking through a dialog
 * is a time rule nothing checks. The dialog renders what these functions
 * say.
 *
 * **Everything here is UTC.** The archives are hourly UTC and the project's
 * timeline is UTC (spec 3), so reading a person's local clock into either
 * would silently shift the whole field. The dialog says so on the labels;
 * this module simply never looks at the local zone.
 */

/** Seconds in an hour. The archives are hourly and so is the range. */
const HOUR = 3600;

/**
 * The most steps one import fetches.
 *
 * The same bound the backend enforces (`ve_app::history::MAX_FETCHED_STEPS`).
 * It is repeated here so the dialog can say "too many" before a minutes-long
 * fetch starts, not so the backend can trust it: the refusal that matters is
 * the one on the Rust side.
 */
export const MAX_FETCHED_STEPS = 240;

/** An archive an import can read, as `ve_zarr::Archive::id` spells it. */
export interface ArchiveChoice {
  /** The identifier the command takes. */
  id: string;
  /** What the checkbox says. */
  label: string;
  /** What that archive holds. */
  detail: string;
}

/** The archives, in the order the import reads them. */
export const ARCHIVES: readonly ArchiveChoice[] = [
  {
    id: "era5-wind",
    label: "ERA5 wind",
    detail: "10 m wind, hourly, 0.25°",
  },
  {
    id: "globcurrent",
    label: "GlobCurrent",
    detail: "total surface current, hourly, 0.25°",
  },
];

/**
 * Reads a `datetime-local` value as UTC seconds, floored to the hour.
 *
 * The control's value has no zone, and this reads it as UTC rather than as
 * local time — which is the whole reason it is not passed to `Date.parse`,
 * which would read it as local.
 */
export function parseUtcHour(value: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::\d{2})?$/.exec(value.trim());
  if (!match) return null;
  const [, y, mo, d, h] = match;
  const year = Number(y);
  const month = Number(mo);
  const day = Number(d);
  const hour = Number(h);
  if (month < 1 || month > 12 || day < 1 || day > 31 || hour > 23) return null;
  const seconds = Date.UTC(year, month - 1, day, hour) / 1000;
  // Date.UTC rolls an impossible day over into the next month; a value the
  // user could not have meant should be refused, not silently moved.
  const back = new Date(seconds * 1000);
  if (back.getUTCMonth() !== month - 1 || back.getUTCDate() !== day) return null;
  return seconds;
}

/** Writes UTC seconds back as a `datetime-local` value, on the hour. */
export function formatUtcHour(unixS: number): string {
  const at = new Date(Math.floor(unixS / HOUR) * HOUR * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())}` +
    `T${pad(at.getUTCHours())}:00`
  );
}

/** What a range is, and what is wrong with it. */
export interface RangeState {
  /** Start in UTC seconds, or null when the field does not parse. */
  start: number | null;
  /** End in UTC seconds, or null when the field does not parse. */
  end: number | null;
  /** Steps that will be fetched from each archive. Zero when unreadable. */
  steps: number;
  /** Why the import cannot run, in a sentence, or null when it can. */
  problem: string | null;
}

/**
 * How many of a project's steps a range covers.
 *
 * The project's first step is the range's first hour (spec 4.8), so the times
 * it can show are the start and every `stepHours` after it, at most
 * `stepCount` of them. This is the count the import fetches: an hour between
 * two steps is never drawn and is never downloaded.
 *
 * Both ends are inclusive, matching the backend: a start and an end in the
 * same hour is one step of data, not none.
 */
export function stepsInRange(
  start: number,
  end: number,
  stepHours: number,
  stepCount: number,
): number {
  if (end < start) return 0;
  const stride = Math.max(1, Math.trunc(stepHours)) * HOUR;
  const from = Math.floor(start / HOUR) * HOUR;
  if (end < from) return 0;
  return Math.min(stepCount, Math.floor((end - from) / stride) + 1);
}

/** Judges a range, a set of archives, and the project's own step. */
export function rangeState(
  startValue: string,
  endValue: string,
  archives: readonly string[],
  stepHours: number,
  stepCount: number,
): RangeState {
  const start = parseUtcHour(startValue);
  const end = parseUtcHour(endValue);
  if (start === null || end === null) {
    return { start, end, steps: 0, problem: "Both dates need a day and an hour." };
  }
  if (end < start) {
    return { start, end, steps: 0, problem: "The end is before the start." };
  }
  const steps = stepsInRange(start, end, stepHours, stepCount);
  if (steps === 0) {
    return { start, end, steps, problem: "That range holds none of the project's steps." };
  }
  if (steps > MAX_FETCHED_STEPS) {
    return {
      start,
      end,
      steps,
      problem: `${steps} steps is more than one import fetches. Ask for ${MAX_FETCHED_STEPS} or fewer.`,
    };
  }
  if (archives.length === 0) {
    return { start, end, steps, problem: "Choose at least one archive." };
  }
  return { start, end, steps, problem: null };
}

/** How far back the dialog opens, in days. */
const DEFAULT_LAG_DAYS = 14;

/**
 * The range the dialog opens on: a day, ending a fortnight back.
 *
 * Both archives trail real time. ERA5's final stream runs months behind, with
 * the preliminary ERA5T filling in behind it, and GlobCurrent's near-real-time
 * stream runs days behind. A range ending *now* is one neither archive has.
 *
 * A fortnight rather than a week, because ERA5's own attributes overstate what
 * it holds: measured on 2026-09-07 they advertised hours through 2026-09-01
 * while the last written chunk was 2026-08-30, so a week back landed in the
 * gap and the import failed on its first hour. The default has to be a range
 * that works, and the margin costs nothing — any range the user prefers is two
 * fields away.
 */
export function defaultRange(nowUnixS: number): { start: string; end: string } {
  const end = Math.floor(nowUnixS / HOUR) * HOUR - DEFAULT_LAG_DAYS * 24 * HOUR;
  return { start: formatUtcHour(end - 23 * HOUR), end: formatUtcHour(end) };
}
