/**
 * Playback and readiness, as rules (spec.md 9.4, 9.5).
 *
 * Everything the timeline decides that is not drawing: which state a step's
 * readiness is in, whether playback may advance, what a step is called on the
 * ruler. Pure, so the rules that make playback hold rather than stutter can be
 * tested without a clock, a pool or a screen.
 */

import type { TimelineReadiness } from "../generated/TimelineReadiness";

/**
 * How ready one step is, as the ruler shows it (spec.md 9.5).
 *
 * `stale` is the state that needs memory: a step that *was* solid at an earlier
 * revision and is not at this one has been invalidated by an edit, and that is
 * worth telling apart from a step that was never rendered. The backend cannot
 * know it — it only reports what the cache holds now — so the frontend keeps
 * the memory.
 */
export type StepState = "empty" | "partial" | "solid" | "stale";

/** What the timeline remembers between readiness reports. */
export interface ReadinessMemory {
  /** For each step, the latest revision at which it was seen solid. */
  solidAt: Map<number, number>;
}

/** A fresh memory: nothing has been seen solid yet. */
export function freshMemory(): ReadinessMemory {
  return { solidAt: new Map() };
}

/**
 * Classifies every step of a report, updating the memory.
 *
 * A step seen solid is remembered with the report's revision. A step that is
 * not solid, but was at an *earlier* revision, is stale until it is solid
 * again — at which point the memory moves on and stale is forgotten.
 */
export function classify(
  report: TimelineReadiness,
  memory: ReadinessMemory,
): { states: StepState[]; progress: number[] } {
  const states: StepState[] = [];
  const progress: number[] = [];
  for (const entry of report.steps) {
    const solid = entry.total > 0 && entry.ready === entry.total;
    const fraction = entry.total > 0 ? entry.ready / entry.total : 0;
    progress.push(fraction);
    if (solid) {
      memory.solidAt.set(entry.step, report.revision);
      states.push("solid");
      continue;
    }
    const seen = memory.solidAt.get(entry.step);
    if (seen !== undefined && seen < report.revision) {
      states.push("stale");
    } else if (entry.ready === 0) {
      states.push("empty");
    } else {
      states.push("partial");
    }
  }
  return { states, progress };
}

/**
 * The step playback would move to, or `null` at the end without looping.
 */
export function nextStep(step: number, last: number, loop: boolean): number | null {
  if (step < last) return step + 1;
  return loop ? 0 : null;
}

/**
 * Whether playback may advance into `next` (spec.md 9.4).
 *
 * Only into a frame that is ready. Anything else — partial, empty, stale — and
 * playback holds where it is and shows that it is buffering, rather than
 * showing the previous revision's tiles dimmed under a step they no longer
 * belong to. A state list shorter than the step count is a report for another
 * timeline; nothing is ready in it.
 */
export function mayAdvanceInto(next: number, states: readonly StepState[]): boolean {
  return states[next] === "solid";
}

/**
 * One tick of the playback clock.
 *
 * Playback advances one step per interval at the user's rate, not in real time
 * (spec.md 9.4). `elapsedMs` is how long since the last advance; the answer is
 * where to be now and whether the wait was for readiness rather than the
 * clock. Deliberately not "advance as many steps as elapsed": a stall — a long
 * render, a suspended tab — must resume at the next step, not skip a dozen.
 */
export function tick(
  step: number,
  last: number,
  loop: boolean,
  rateStepsPerSecond: number,
  elapsedMs: number,
  states: readonly StepState[],
): { step: number; advanced: boolean; buffering: boolean; finished: boolean } {
  const interval = 1000 / Math.max(rateStepsPerSecond, 0.01);
  if (elapsedMs < interval) {
    return { step, advanced: false, buffering: false, finished: false };
  }
  const next = nextStep(step, last, loop);
  if (next === null) {
    return { step, advanced: false, buffering: false, finished: true };
  }
  if (!mayAdvanceInto(next, states)) {
    return { step, advanced: false, buffering: true, finished: false };
  }
  return { step: next, advanced: true, buffering: false, finished: false };
}

/** The forecast hour of a step: `+0 h`, `+3 h`, ... */
export function forecastLabel(step: number, stepHours: number): string {
  return `+${step * stepHours} h`;
}

/**
 * The absolute time of a step, once a start is set (spec.md 9.1).
 *
 * UTC only, and formatted here rather than by the locale: a forecast is read
 * against charts and files that are all in UTC, and a ruler that quietly showed
 * local time would be wrong by whatever the machine's offset happened to be.
 * `null` when no start time is set.
 */
export function utcLabel(
  step: number,
  stepHours: number,
  startUnixS: number | null,
): string | null {
  if (startUnixS === null) return null;
  const at = new Date((startUnixS + step * stepHours * 3600) * 1000);
  const two = (n: number) => String(n).padStart(2, "0");
  const day = two(at.getUTCDate());
  const month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][
    at.getUTCMonth()
  ];
  return `${day} ${month} ${two(at.getUTCHours())}:${two(at.getUTCMinutes())}Z`;
}

/**
 * Which ticks carry a label, so labels never overlap.
 *
 * Every tick is drawn; every `every`-th is labelled, where `every` is the
 * smallest step count that keeps labels at least `minPx` apart. Chosen from a
 * ladder of "nice" counts so the labelled steps are the ones a reader would
 * expect — every 2, every 4, every 8 — rather than every 7.
 */
export function labelEvery(pxPerStep: number, minPx = 56): number {
  for (const every of [1, 2, 4, 5, 8, 10, 20, 24, 40, 48, 80, 120]) {
    if (pxPerStep * every >= minPx) return every;
  }
  return 240;
}

/**
 * A step from a pointer position on the ruler, clamped to the timeline.
 *
 * Steps are cells of `pxPerStep`, so the position rounds to the nearest cell
 * centre — which is what makes a scrub land on the step under the pointer
 * rather than the one to its left.
 */
export function stepAt(x: number, pxPerStep: number, last: number): number {
  return Math.min(last, Math.max(0, Math.round(x / pxPerStep - 0.5)));
}

/**
 * A dragged key's destination, from where it was grabbed and how far the
 * pointer has moved. Constrained to whole steps (spec.md 9.3) and to the
 * timeline.
 */
export function draggedStep(from: number, deltaPx: number, pxPerStep: number, last: number): number {
  return Math.min(last, Math.max(0, from + Math.round(deltaPx / pxPerStep)));
}

/**
 * The tile addresses the timeline asks readiness for.
 *
 * The map lists a tile once per world copy it draws (dateline wrapping); the
 * cache holds it once, so the copies collapse here and the counts stay honest.
 */
export function uniqueTiles(
  tiles: ReadonlyArray<{ z: number; x: number; y: number }>,
): Array<{ z: number; x: number; y: number }> {
  const seen = new Set<string>();
  const out: Array<{ z: number; x: number; y: number }> = [];
  for (const tile of tiles) {
    const key = `${tile.z}/${tile.x}/${tile.y}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ z: tile.z, x: tile.x, y: tile.y });
  }
  return out;
}
