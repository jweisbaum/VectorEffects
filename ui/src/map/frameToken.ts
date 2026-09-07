/**
 * The token that names a frame of tiles.
 *
 * A tile's address is `<frame>/<z>/<x>/<y>`, and everything before the `z` is
 * one opaque string as far as the tile cache is concerned: it keys textures by
 * the backend's content hash and never reads the token itself. So the token is
 * the whole of what the map and the backend have to agree about, and it is
 * built and read in this one module. Two copies of the shape — one writing it
 * and one parsing it — would drift, and the failure would be silent: tiles of
 * the wrong scene, correctly cached, under a plausible address.
 *
 * Two shapes:
 *
 * - `<revision>/<step>` — the whole stack, which is what the map draws.
 * - `without/<layer>/<revision>/<step>` — the same, with one layer left out
 *   (spec.md 6.2, M40). What the map draws back into the hole an eraser's
 *   live remove opens, so a stroke takes one layer's contribution away rather
 *   than the whole composite.
 */

/** A frame token, read back. */
export interface FrameToken {
  revision: number;
  step: number;
  /** The layer left out, or null for the whole stack. */
  without: number | null;
}

/** The whole stack at a step. */
export function frameToken(revision: number, step: number): string {
  return `${revision}/${step}`;
}

/** The stack at a step with one layer left out. */
export function beneathToken(revision: number, step: number, layer: number): string {
  return `without/${layer}/${frameToken(revision, step)}`;
}

/**
 * Reads a token back, or null if it is not one.
 *
 * Returning null rather than guessing: a token that does not parse would
 * otherwise become a request for revision `NaN`, which the backend answers
 * with a refusal and the map retries forever.
 */
export function parseFrameToken(token: string): FrameToken | null {
  const parts = token.split("/");
  const prefixed = parts[0] === "without";
  const without = prefixed ? Number(parts[1]) : null;
  const rest = prefixed ? parts.slice(2) : parts;
  if (rest.length !== 2 || (prefixed && !Number.isInteger(without))) return null;
  const revision = Number(rest[0]);
  const step = Number(rest[1]);
  if (!Number.isFinite(revision) || !Number.isInteger(step)) return null;
  return { revision, step, without };
}
