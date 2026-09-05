/**
 * What long-running work is in flight, for the status bar's spinner.
 *
 * A counter with labels, outside React like the hint store: the ipc layer
 * marks the commands that can take seconds — a GRIB or image import, a
 * project opening or saving, an export, a capture's bake — and the one
 * spinner subscribes. Callers do nothing; the command's name is enough.
 */

import { useSyncExternalStore } from "react";

export interface BusySnapshot {
  /** What is running, in the order it started. Empty when nothing is. */
  labels: readonly string[];
}

let snapshot: BusySnapshot = { labels: [] };
const listeners = new Set<() => void>();

function publish(labels: readonly string[]) {
  snapshot = { labels };
  for (const listener of listeners) listener();
}

/** Marks work as begun; the returned function marks it done. */
export function beginBusy(label: string): () => void {
  publish([...snapshot.labels, label]);
  let ended = false;
  return () => {
    if (ended) return;
    ended = true;
    const index = snapshot.labels.indexOf(label);
    if (index === -1) return;
    publish([...snapshot.labels.slice(0, index), ...snapshot.labels.slice(index + 1)]);
  };
}

/** Whether anything long is running. */
export function isBusy(state: BusySnapshot): boolean {
  return state.labels.length > 0;
}

export function useBusy(): BusySnapshot {
  return useSyncExternalStore(subscribe, () => snapshot);
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** For tests. */
export function currentBusy(): BusySnapshot {
  return snapshot;
}
