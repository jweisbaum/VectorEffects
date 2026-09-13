/** One probe at a time, with one coalesced refresh and no obsolete completions. */
export class ReadinessPoller<T> {
  private running = false;
  private pending = false;
  private disposed = false;

  constructor(private readonly fetch: () => Promise<T>, private readonly accept: (value: T) => void) {}

  request = (): void => {
    if (this.disposed) return;
    if (this.running) { this.pending = true; return; }
    this.running = true;
    void this.fetch().then((value) => {
      if (!this.disposed) this.accept(value);
    }).catch(() => undefined).finally(() => {
      this.running = false;
      if (this.pending && !this.disposed) {
        this.pending = false;
        this.request();
      }
    });
  };

  dispose(): void { this.disposed = true; }
}
