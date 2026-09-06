/**
 * Where a dragged layer lands (spec.md 4.3, M29).
 *
 * The panel shows layers top-first and the document keeps them bottom-first,
 * so "above" on screen is a *higher* index. A drop names the row it landed
 * on and which half of it, and this turns that into the index the document's
 * `MoveLayer` wants — which removes the layer first, so a target above the
 * source has already moved down by one when the layer is put back.
 */

/** The document index a layer at `from` moves to when dropped above or below the layer at `target`. */
export function layerDropIndex(from: number, target: number, above: boolean): number {
  if (from === target) return from;
  if (above) return from < target ? target : target + 1;
  return from < target ? target - 1 : target;
}
