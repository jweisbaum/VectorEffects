/**
 * The rules behind an imported layer's frame marks (spec.md 4.8, M20).
 *
 * Small enough to be inline in `Timeline.tsx` and branchy enough to be worth
 * asserting: which of three things a mark is, and which steps a shift-click
 * takes in.
 */
import type { GribStepView } from "../generated/GribStepView";

/** What a step's mark is. */
export type MarkKind =
  /** The file's own message, shown. */
  | "file"
  /** A message pasted here from another step. */
  | "pasted"
  /** The file has a message here and the user hid it. */
  | "hidden"
  /** Nothing to draw: the file says nothing about this time. */
  | "none";

/**
 * A pasted frame outranks the file's own message, because that is what
 * pasting onto a covered step means; a hidden one is the file's message
 * being deliberately not shown, which is why it is drawn rather than blank.
 */
export function markKind(frame: GribStepView): MarkKind {
  if (frame.source !== null) return "pasted";
  if (frame.hidden) return "hidden";
  return frame.in_file ? "file" : "none";
}

/**
 * The steps a shift-click takes in: everything between the anchor and the
 * click, either way round, both ends included.
 *
 * Every step in the run, not only the marked ones — a run copied from a
 * sparse file keeps its spacing when it is pasted, and it can only do that if
 * the gaps are part of it. The backend drops the empty ones from the copy.
 */
export function runBetween(anchor: number, step: number): number[] {
  const lo = Math.min(anchor, step);
  const hi = Math.max(anchor, step);
  return Array.from({ length: hi - lo + 1 }, (_, k) => lo + k);
}

/**
 * Where a copied run lands when pasted at `at`, keeping its spacing, with the
 * steps that fall past the end dropped rather than clamped.
 *
 * The backend does this too and is the authority; this is what lets the
 * timeline say what a paste will do before it happens.
 */
export function pastedRun(steps: number[], at: number, last: number): number[] {
  if (steps.length === 0) return [];
  const first = Math.min(...steps);
  return steps
    .map((source) => at + (source - first))
    .filter((step) => step <= last);
}
