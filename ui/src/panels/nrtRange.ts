/**
 * The period a near-real-time import asks for, and what it will cost
 * (spec 4.10, M89).
 *
 * Kept apart from the dialog for the reason `historyRange.ts` is: every rule
 * here is a rule about *times*, and a time rule that is only checked by
 * clicking through a dialog is a time rule nothing checks. The dialog renders
 * what these functions say.
 *
 * **This mirrors the backend and does not replace it.** `ve_app::nrt::period`
 * and `wanted_times` decide what is fetched; the arithmetic is repeated here
 * so the dialog can say how far back the period reaches and how many
 * downloads it is before a minutes-long fetch starts. The request carries a
 * number of days and nothing computed here, so a disagreement would be a
 * wrong sentence in the dialog and never a wrong import.
 *
 * **Everything here is UTC.** The products are stamped in UTC and the
 * project's timeline is UTC (spec 3), so "today" is the UTC day and this
 * module never looks at the local zone.
 */

import { msg } from "../i18n";

/** Seconds in an hour. */
const HOUR = 3600;
/** Seconds in a day. */
const DAY = 24 * HOUR;

/**
 * The most steps a timeline holds, and so the most a period may need.
 *
 * The same bound the backend enforces (`ve_app::history::MAX_FETCHED_STEPS`),
 * repeated so the dialog can cap the number of days rather than let the
 * import be refused: the refusal that matters is the one on the Rust side.
 */
export const MAX_STEPS = 240;

/** The days the dialog opens on, where the timeline's step allows that many. */
export const DEFAULT_DAYS = 3;

/** Which field a product is part of, and so which heading it is listed under. */
export type NrtGroup = "current" | "wind";

/** A product an import can fetch, as `ve_zarr::Product::id` spells it. */
export interface NrtProduct {
  /** The identifier the command takes. */
  id: string;
  /** The heading it is listed under. */
  group: NrtGroup;
  /**
   * What the checkbox says. English: pass it through `t` to show it. A
   * product's own name is a proper noun and is not marked with `msg`, so it
   * reads the same in every language — the way "GlobCurrent" does.
   */
  label: string;
  /** What that product holds. English, via `msg`: pass it through `t` to show it. */
  detail: string;
  /** Hours between the product's times: 1 for an hourly product, 24 for a daily one. */
  periodHours: number;
  /** Roughly what one fetched time takes on disk, for the cost line. */
  megabytesPerTime: number;
}

/** The products, in the order the import reads them. */
export const NRT_PRODUCTS: readonly NrtProduct[] = [
  {
    id: "multiobs",
    group: "current",
    label: "Copernicus MULTIOBS",
    detail: msg("total surface current, hourly, 0.25°"),
    periodHours: 1,
    megabytesPerTime: 2.8,
  },
  {
    id: "duacs",
    group: "current",
    label: "Copernicus DUACS",
    detail: msg("geostrophic surface current, daily"),
    periodHours: 24,
    megabytesPerTime: 2.8,
  },
  {
    id: "wind-l4",
    group: "wind",
    label: msg("Copernicus L4 wind"),
    detail: msg("10 m wind, hourly"),
    periodHours: 1,
    megabytesPerTime: 4.2,
  },
  {
    id: "ascat",
    group: "wind",
    label: "ASCAT Metop-B/C",
    detail: msg("scatterometer 10 m wind, daily swaths, 0.25°"),
    periodHours: 24,
    megabytesPerTime: 4.2,
  },
];

/** The headings the products are listed under, in the order they are shown. */
export const NRT_GROUPS: readonly { id: NrtGroup; heading: string }[] = [
  { id: "current", heading: msg("Currents") },
  { id: "wind", heading: msg("Wind") },
];

/**
 * The line the products' licence asks to be shown wherever they are used.
 *
 * Deliberately not marked with `msg`: it is an attribution, worded by the
 * Copernicus Marine Service, and is shown as written in every language. The
 * same sentence is `ve_zarr::Product::credit` on the Rust side.
 */
export const COPERNICUS_CREDIT = "Generated using E.U. Copernicus Marine Service Information";

/** The span an import covers and what the timeline needs to show it. */
export interface NrtPeriod {
  /** 00:00 UTC, the asked number of days before the UTC day of "now". */
  startUnixS: number;
  /** The hour "now" is in. */
  endUnixS: number;
  /** Steps from the start to the last one not past the end. */
  steps: number;
}

/**
 * The period `days` back from `nowUnixS`, on a timeline of `stepHours`.
 *
 * It starts on a midnight so a daily product has whole days and the
 * timeline's labels start on one; it ends on the current hour because nothing
 * later exists. Both ends count, so the steps are the gaps plus one.
 *
 * Unlike the backend's, this does not refuse a period too long for a
 * timeline: it reports the count, and `maxDays` is what keeps the dialog from
 * asking for one.
 */
export function periodOf(nowUnixS: number, days: number, stepHours: number): NrtPeriod {
  const today = Math.floor(nowUnixS / DAY) * DAY;
  const startUnixS = today - days * DAY;
  const endUnixS = Math.floor(nowUnixS / HOUR) * HOUR;
  const stride = Math.max(1, Math.trunc(stepHours)) * HOUR;
  return {
    startUnixS,
    endUnixS,
    steps: Math.floor((endUnixS - startUnixS) / stride) + 1,
  };
}

/**
 * The most days a timeline of `stepHours` can hold a period of.
 *
 * It depends on the hour as well as the step: the period runs from a midnight
 * to *now*, so late in the day there is nearly a day more of it than early,
 * and an hourly timeline that takes nine days in the afternoon takes nine in
 * the morning too but not ten at either. Never less than one — a single day
 * is at most forty-eight hourly steps, which every timeline holds.
 */
export function maxDays(nowUnixS: number, stepHours: number): number {
  let days = 1;
  while (periodOf(nowUnixS, days + 1, stepHours).steps <= MAX_STEPS) days += 1;
  return days;
}

/**
 * A typed number of days as one the import can be asked for: whole, at least
 * one, and no more than `most`.
 */
export function clampDays(days: number, most: number): number {
  if (!Number.isFinite(days)) return 1;
  return Math.min(Math.max(1, most), Math.max(1, Math.round(days)));
}

/**
 * How many times of one product an import fetches: those a step of the
 * timeline can show, each once.
 *
 * A product is walked at the coarser of its own period and the timeline's
 * step — a daily product at its midnights however fine the timeline, an
 * hourly one at every third hour on a three-hourly timeline — from the
 * period's start, to its end, and no further than the timeline's last step.
 * The count of `ve_app::nrt::wanted_times`, walked the same way rather than
 * derived, so the two cannot disagree at an edge.
 */
export function wantedTimes(
  period: NrtPeriod,
  stepHours: number,
  stepCount: number,
  productPeriodHours: number,
): number {
  const step = Math.max(1, Math.trunc(stepHours)) * HOUR;
  const stride = Math.max(step, productPeriodHours * HOUR);
  const timelineEnd = period.startUnixS + stepCount * step;
  let count = 0;
  for (
    let time = period.startUnixS;
    time <= period.endUnixS && time < timelineEnd;
    time += stride
  ) {
    count += 1;
  }
  return count;
}

/** What an import will fetch: how many downloads, and roughly how much disk. */
export interface NrtCost {
  /** Times fetched, summed over the products chosen. */
  downloads: number;
  /** Whole megabytes on disk, approximately. */
  megabytes: number;
}

/**
 * The cost of fetching the chosen products over a period.
 *
 * The download count is what the wait is proportional to, and the size is
 * what the project's folder grows by: fetched times are written to disk so
 * the project opens offline afterwards. An identifier that names no product
 * costs nothing — the backend is what refuses it.
 */
export function costOf(
  productIds: readonly string[],
  period: NrtPeriod,
  stepHours: number,
  stepCount: number,
): NrtCost {
  let downloads = 0;
  let megabytes = 0;
  for (const product of NRT_PRODUCTS) {
    if (!productIds.includes(product.id)) continue;
    const times = wantedTimes(period, stepHours, stepCount, product.periodHours);
    downloads += times;
    megabytes += times * product.megabytesPerTime;
  }
  return { downloads, megabytes: Math.round(megabytes) };
}
