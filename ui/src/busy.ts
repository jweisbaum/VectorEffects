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

/**
 * Marks the wait while a native file dialog is up (M74).
 *
 * The spinner is started at a *boundary*, never by a feature component: the
 * ipc layer marks a command by name, and this marks a dialog. Both are single
 * chokepoints — there is one `pickProjectToOpen` and every caller goes through
 * it — which is the property that keeps two callers of one thing from showing
 * two spinners' worth of nothing.
 *
 * It exists because the command's own marker starts too late to answer the
 * click. Opening a project is a decision prompt, then a native dialog, and
 * only then `open_project`: the spinner appeared after all of it, so a click
 * on Open produced no sign that anything had happened until the file had been
 * chosen. The dialog is part of the operation and is marked as such.
 */
export async function whileChoosing<T>(label: string, choose: () => Promise<T>): Promise<T> {
  const done = beginBusy(label);
  try {
    return await choose();
  } finally {
    done();
  }
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
