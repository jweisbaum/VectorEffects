/** Opt-in, bounded diagnostics. No tile logging or readbacks in the frame loop. */
const LIMIT = 20_000;
let started: number | null = null;
let stopped: number | null = null;
let lastFrame: string | null = null;
let frames: Array<{ frame: string; at: number }> = [];
let counts: Record<string, number> = {};
let times: Record<string, number[]> = {};

export function playbackCount(name: string, count = 1): void {
  if (started !== null && stopped === null) counts[name] = (counts[name] ?? 0) + count;
}

export function playbackTime(name: string, ms: number): void {
  if (started === null || stopped !== null) return;
  const samples = times[name] ?? (times[name] = []);
  if (samples.length < LIMIT) samples.push(ms);
}

/**
 * TEMPORARY (2026-09-21): the latest preparation decision, always recorded.
 *
 * Diagnosing "preparing playback 2/3" under an all-solid ruler. Unlike the
 * counters above this is not gated on `start()`, because the question is
 * what the steady state looks like rather than what a playback run did.
 * Read it with `__vePlayback.preparation()`. Remove with the fix.
 */
let preparation: unknown = null;
export function playbackPreparation(detail: unknown): void {
  preparation = detail;
}

export function playbackPresented(frame: string): void {
  if (started === null || stopped !== null || frame === lastFrame) return;
  lastFrame = frame;
  if (frames.length < LIMIT) frames.push({ frame, at: performance.now() - started });
}

const percentile = (values: number[], fraction: number) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))] ?? 0;
};

export const playbackMetrics = {
  start() {
    started = performance.now();
    stopped = null;
    lastFrame = null;
    frames = [];
    counts = {};
    times = {};
  },
  snapshot() {
    const durationMs = started === null ? 0 : (stopped ?? performance.now()) - started;
    const intervals = frames.slice(1).map((frame, i) => frame.at - frames[i]!.at);
    const span = frames.length < 2 ? 0 : frames.at(-1)!.at - frames[0]!.at;
    return {
      durationMs,
      displayedFps: span > 0 ? (frames.length - 1) * 1000 / span : 0,
      intervalP50: percentile(intervals, 0.5),
      intervalP95: percentile(intervals, 0.95),
      maxInterval: intervals.length ? Math.max(...intervals) : 0,
      frames: [...frames],
      counts: { ...counts },
      timings: Object.fromEntries(Object.entries(times).map(([name, samples]) =>
        [name, { count: samples.length, p50: percentile(samples, 0.5), p95: percentile(samples, 0.95) }])),
    };
  },
  stop() {
    stopped = performance.now();
    return this.snapshot();
  },
  /** TEMPORARY (2026-09-21): see `playbackPreparation`. */
  preparation() {
    return preparation;
  },
};

declare global {
  interface Window { __vePlayback: typeof playbackMetrics }
}
if (typeof window !== "undefined") window.__vePlayback = playbackMetrics;
