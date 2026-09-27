/**
 * The dependency-free half of the translation API: marking a string and
 * filling its placeholders. Code that runs in a worker (the projection mesh)
 * imports this rather than `./index`, which would carry React and every
 * catalogue into the worker's bundle.
 */

export type Params = Record<string, string | number>;

/** Fills `{name}` placeholders. A placeholder with no value is left as written. */
export function interpolate(text: string, params?: Params): string {
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole);
}

/**
 * Marks a string for translation without translating it yet: for tables built
 * once at module load and shown later, which pass the entry through `t` at
 * render time. The coverage test reads `msg(...)` exactly as it reads `t(...)`.
 */
export function msg<T extends string>(text: T): T {
  return text;
}

/**
 * An error whose message the interface translates when it shows it: the key
 * is English, marked with `msg`, and the parameters fill its placeholders.
 * `message` is the English, for logs.
 */
export class TranslatableError extends Error {
  constructor(readonly key: string, readonly params?: Params) {
    super(interpolate(key, params));
  }
}
