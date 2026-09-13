/** A sequential playback clock. Normal display jitter is carried, stalls are not. */
export class PlaybackClock {
  private deadline: number;
  private interval: number;
  private blocked = false;

  constructor(now: number, rate: number) {
    this.interval = 1000 / Math.max(0.01, rate);
    this.deadline = now + this.interval;
  }

  due(now: number, rate: number): boolean {
    const interval = 1000 / Math.max(0.01, rate);
    if (interval !== this.interval) {
      this.interval = interval;
      this.deadline = now + interval;
      this.blocked = false;
    }
    // Subtraction at a floating-point boundary must not cost a refresh.
    return now + 0.001 >= this.deadline;
  }

  hold(): void {
    this.blocked = true;
  }

  /** Called only after the next frame has actually been drawn. */
  advance(now: number): void {
    this.deadline = this.blocked || now - this.deadline >= this.interval
      ? now + this.interval
      : this.deadline + this.interval;
    this.blocked = false;
  }
}
