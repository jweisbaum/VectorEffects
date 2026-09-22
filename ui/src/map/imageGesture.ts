/** One native write at a time, retaining only the latest pointer position.
 * Release drains that position before ending undo coalescing. A subsequent
 * drag waits on `done`, so a late release cannot end the new drag's gesture.
 */
export function imageGesture<T>(previous: Promise<void>, write: (point: T) => Promise<unknown>,
  end: () => Promise<unknown>, report: (error: unknown) => void) {
  let queued: T | null = null;
  let running = false;
  let released = false;
  let complete!: () => void;
  const done = new Promise<void>((resolve) => { complete = resolve; });
  async function drain() {
    if (running) return;
    running = true;
    await previous;
    while (queued !== null) {
      const point = queued;
      queued = null;
      try { await write(point); } catch (error) { report(error); }
    }
    if (released) {
      try { await end(); } catch (error) { report(error); }
      complete();
    }
    running = false;
  }
  return {
    done,
    push(point: T) { if (!released) { queued = point; void drain(); } },
    finish() { if (!released) { released = true; void drain(); } },
  };
}
