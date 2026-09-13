/**
 * The status bar's one line: the tool's hint, or the last error, whichever
 * is newer (M25).
 *
 * A store outside React, the readout store's shape: every panel and the map
 * write to it, and only the status bar's span subscribes — so a hint set on
 * a pointer report, or an error from a refused write, re-renders one span
 * rather than the shell. Before this every panel kept its own error line and
 * the map its own overlay, and the capture bar's "draw a region first" sat
 * in the toolbar where the tool's options belong.
 */

import { useSyncExternalStore } from "react";

export interface HintSnapshot {
  /** What the tool in hand would like the user to know, if anything. */
  hint: string | null;
  /** The last error, until the next hint or a clear. */
  error: string | null;
  /**
   * What a bug report would want and a reader would not: the error's `kind`
   * discriminant (M59). Shown as the line's tooltip, never in the line.
   */
  errorKind: string | null;
  /**
   * What the map is busy with — tiles still rendering — shown ahead of the
   * hint (M27). It used to sit in the title bar among the view controls,
   * where a count that comes and goes on every edit pulled the eye; the
   * status bar is where the eye already goes for what is happening.
   */
  activity: string | null;
  retry?: (() => void) | null;
}

let snapshot: HintSnapshot = { hint: null, error: null, errorKind: null, activity: null };
const listeners = new Set<() => void>();

function publish(next: HintSnapshot) {
  if (
    next.hint === snapshot.hint &&
    next.error === snapshot.error &&
    next.errorKind === snapshot.errorKind &&
    next.activity === snapshot.activity && next.retry === snapshot.retry
  ) {
    return;
  }
  snapshot = next;
  for (const listener of listeners) listener();
}

/**
 * Sets the tool's hint. The error stands until the next hint *change*: a
 * refusal is more recent than the hint it interrupted, and a hint that has
 * not changed has nothing new to say over it.
 */
export function setHint(hint: string | null): void {
  const keep = hint === snapshot.hint || !!snapshot.retry;
  publish({
    ...snapshot,
    hint,
    error: keep ? snapshot.error : null,
    errorKind: keep ? snapshot.errorKind : null,
  });
}

/**
 * Reports an error, which shows in place of the hint until the hint changes.
 *
 * `kind` is the backend's discriminant, kept for the tooltip and never for the
 * line itself (M59).
 */
export function reportError(error: string | null, kind: string | null = null, retry?: () => void): void {
  publish({ ...snapshot, error, errorKind: error === null ? null : kind, retry: error ? retry ?? null : null });
}

/** Consume the action before starting, preventing duplicate downloads. */
export function retryError(): void {
  const retry = snapshot.retry;
  reportError(null);
  retry?.();
}

/** Sets what the map is busy with, or clears it. Independent of the hint and the error. */
export function setActivity(activity: string | null): void {
  publish({ ...snapshot, activity });
}

/** What the status bar shows: the error if there is one, else the hint. */
export function shown(
  state: HintSnapshot,
): { text: string; kind: "error" | "hint"; detail: string | null } | null {
  if (state.error !== null) {
    return { text: state.error, kind: "error", detail: state.errorKind };
  }
  if (state.hint !== null) return { text: state.hint, kind: "hint", detail: null };
  return null;
}

/** The current snapshot, for the one component that renders it. */
export function useHint(): HintSnapshot {
  return useSyncExternalStore(subscribe, () => snapshot);
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** For tests: the snapshot as it stands. */
export function currentHint(): HintSnapshot {
  return snapshot;
}
