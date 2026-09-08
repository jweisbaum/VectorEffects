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
 * Three shapes:
 *
 * - `<revision>/<step>` — the whole stack, which is what the map draws.
 * - `without/<layer>/<revision>/<step>` — the same, with one layer left out
 *   (spec.md 6.2, M40, M44). What fills the hole an eraser's live remove
 *   opens, and what tells every other live edit which pixels belong to the
 *   layer it is aimed at.
 * - `only/<layer>/<revision>/<step>` — that layer by itself (M45). What a
 *   clone stamp reads its source from: the commit samples the layer the
 *   stamp is in, so a preview that read the whole stack showed the wrong
 *   field arriving under the brush.
 */

/** What a frame holds beyond the revision and the step. */
export type FrameScope =
  | { kind: "whole" }
  | { kind: "without"; layer: number }
  | { kind: "only"; layer: number };

/** A frame token, read back. */
export interface FrameToken {
  revision: number;
  step: number;
  scope: FrameScope;
}

/** The whole stack at a step. */
export function frameToken(revision: number, step: number): string {
  return `${revision}/${step}`;
}

/** The stack at a step with one layer left out. */
export function beneathToken(revision: number, step: number, layer: number): string {
  return `without/${layer}/${frameToken(revision, step)}`;
}

/** One layer at a step, by itself. */
export function onlyToken(revision: number, step: number, layer: number): string {
  return `only/${layer}/${frameToken(revision, step)}`;
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
  const head = parts[0];
  const prefixed = head === "without" || head === "only";
  const layer = prefixed ? Number(parts[1]) : null;
  const rest = prefixed ? parts.slice(2) : parts;
  if (rest.length !== 2 || (prefixed && !Number.isInteger(layer))) return null;
  const revision = Number(rest[0]);
  const step = Number(rest[1]);
  if (!Number.isFinite(revision) || !Number.isInteger(step)) return null;
  const scope: FrameScope = !prefixed
    ? { kind: "whole" }
    : { kind: head === "only" ? "only" : "without", layer: layer as number };
  return { revision, step, scope };
}
