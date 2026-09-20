/**
 * What is being opened and how far along it is, for the loading page
 * (spec.md 4.7).
 *
 * A store outside React, the busy store's shape and started at the same
 * boundary: the ipc layer begins an opening by the command's name, so the
 * start screen, the Open button and the automation driver all get the page
 * without any of them asking for it. The backend's `open://progress` events
 * feed it while the files are read; the application ends it once the map has
 * something to show.
 */

import { useSyncExternalStore } from "react";

import type { DocumentTree } from "../generated/DocumentTree";
import type { OpenProgress } from "../generated/OpenProgress";

export interface OpeningSnapshot {
  /** What is being opened, by the name to show for it. Null when nothing is. */
  title: string | null;
  /**
   * `reading` while the command runs; `drawing` once it has answered and the
   * map is fetching its first frame. Rendered is not shown: a page that left
   * when the backend was done would give way to a blank map.
   */
  stage: "reading" | "drawing";
  /** The backend's last report, or null before the first. */
  progress: OpenProgress | null;
}

const IDLE: OpeningSnapshot = { title: null, stage: "reading", progress: null };

let snapshot: OpeningSnapshot = IDLE;
const listeners = new Set<() => void>();

function publish(next: OpeningSnapshot) {
  snapshot = next;
  for (const listener of listeners) listener();
}

/**
 * Marks an opening as begun. The returned function is told how it ended:
 * opened, and the page stays for the map's first frame; refused or failed,
 * and it goes at once, since there will be no map to wait for.
 */
export function beginOpening(title: string): (opened: boolean) => void {
  const mine: OpeningSnapshot = { title, stage: "reading", progress: null };
  publish(mine);
  let ended = false;
  return (opened) => {
    if (ended) return;
    ended = true;
    // A second opening begun over this one owns the page now.
    if (snapshot.title !== mine.title || snapshot.stage !== "reading") return;
    publish(opened ? { ...snapshot, stage: "drawing" } : IDLE);
  };
}

/** A report from the backend. Dropped unless an opening is reading. */
export function reportOpening(progress: OpenProgress): void {
  if (snapshot.title === null || snapshot.stage !== "reading") return;
  publish({ ...snapshot, progress });
}

/** The map is showing the project: the page can go. */
export function finishOpening(): void {
  if (snapshot.title !== null) publish(IDLE);
}

/**
 * The bar, from 0 to 1. The reading is all but the last of it, and the map's
 * first frame is the rest, so the bar is never full while there is a wait.
 */
export function barFraction(state: OpeningSnapshot): number {
  if (state.stage === "drawing") return 0.97;
  const fraction = state.progress?.fraction ?? 0;
  return Math.min(Math.max(fraction, 0), 1) * 0.95;
}

/** The line under the bar: what is being read, and how much of it. */
export function barLabel(state: OpeningSnapshot): string {
  if (state.stage === "drawing") return "Drawing the map";
  const progress = state.progress;
  if (progress === null) return "Reading";
  if (progress.total === 0) return progress.label;
  return `${progress.label} · ${progress.done.toLocaleString()} of ${progress.total.toLocaleString()}`;
}

/**
 * What to say about imported layers whose files could not be read, or null
 * when every one was. The project opens regardless — the user's own work is
 * in its objects — and the layer panel marks each one, but a panel may be
 * closed and a field quietly missing from the map should not be.
 */
export function unreadLayers(tree: DocumentTree): string | null {
  const names = tree.layers
    .filter((layer) => layer.grib !== null && !layer.grib.loaded)
    .map((layer) => layer.name);
  if (names.length === 0) return null;
  const what = names.length === 1 ? `the layer "${names[0]}"` : `${names.length} layers (${names.join(", ")})`;
  return `Opened without ${what}: the imported file is missing or unreadable.`;
}

export function useOpening(): OpeningSnapshot {
  return useSyncExternalStore(subscribe, () => snapshot);
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** For tests. */
export function currentOpening(): OpeningSnapshot {
  return snapshot;
}
