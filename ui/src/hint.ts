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
}

let snapshot: HintSnapshot = { hint: null, error: null };
const listeners = new Set<() => void>();

function publish(next: HintSnapshot) {
  if (next.hint === snapshot.hint && next.error === snapshot.error) return;
  snapshot = next;
  for (const listener of listeners) listener();
}

/**
 * Sets the tool's hint. The error stands until the next hint *change*: a
 * refusal is more recent than the hint it interrupted, and a hint that has
 * not changed has nothing new to say over it.
 */
export function setHint(hint: string | null): void {
  publish({ hint, error: hint === snapshot.hint ? snapshot.error : null });
}

/** Reports an error, which shows in place of the hint until the hint changes. */
export function reportError(error: string | null): void {
  publish({ ...snapshot, error });
}

/** What the status bar shows: the error if there is one, else the hint. */
export function shown(state: HintSnapshot): { text: string; kind: "error" | "hint" } | null {
  if (state.error !== null) return { text: state.error, kind: "error" };
  if (state.hint !== null) return { text: state.hint, kind: "hint" };
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
