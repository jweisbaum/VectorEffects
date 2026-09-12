/**
 * Keeping a layer selected (spec.md 6.1, M75).
 *
 * A layer is always active while a project is open, so that a tool picked up
 * has somewhere to draw without the user having chosen one first. `null` still
 * means the top of the stack everywhere that resolves it — `creation_layer` on
 * the Rust side, `targetLayer` on this one — but that fallback is now the
 * safety net for the moment before the tree lands rather than the ordinary
 * case.
 */

/** As much of a layer as choosing one needs. */
export interface LayerChoice {
  id: number;
  visible: boolean;
}

/** A layer and what is in it, for following a selection. */
export interface LayerContents {
  id: number;
  objects: ReadonlyArray<{ id: number }>;
}

/**
 * The layer that should be made active, or `null` to leave it alone.
 *
 * Chosen when nothing is active, and when what is active has gone — a layer
 * can be deleted out from under the selection, and a dangling id is the same
 * problem as none at all.
 *
 * **The topmost *visible* layer**, not simply the topmost. A gesture aimed at
 * a hidden layer is refused (§8.1, M68), so landing the user on one by default
 * would answer their first stroke with a refusal about a layer they never
 * picked. The list is bottom-first, as the document is, so the topmost is the
 * last of it.
 *
 * **A hidden layer the user chose is left alone.** Only absence is corrected,
 * never a deliberate choice: the refusal that follows is then about something
 * they did, and says so.
 *
 * With no visible layer at all there is nothing to choose and it stays as it
 * is — every gesture is going to be refused whatever this returns, and a
 * layer nobody picked would make the refusal harder to understand, not easier.
 */
export function layerToActivate(
  active: number | null,
  layers: readonly LayerChoice[],
): number | null {
  if (active !== null && layers.some((layer) => layer.id === active)) return null;
  for (let index = layers.length - 1; index >= 0; index--) {
    const layer = layers[index];
    if (layer?.visible) return layer.id;
  }
  return null;
}

/**
 * The layer a selection implies, or `null` to leave the active one alone
 * (M83).
 *
 * Selecting an object in the layer panel already activates its layer — the two
 * happen in one gesture, so nothing is left disagreeing. Selecting one **on
 * the map** did not: the selection moved and the active layer stayed where it
 * was, so the handles described an object in one layer while the next stroke
 * would land in another, and the properties panel and the marquee's scope
 * disagreed with what was plainly selected.
 *
 * **Only when the active layer holds none of the selection.** A selection can
 * span layers (§8.2's cross-layer modifier), and pulling the active layer to
 * the first of them would move it out from under a selection it already
 * describes. Holding *any* of what is selected is enough to be the right
 * layer; holding none of it is what makes it the wrong one.
 *
 * The first selected object's layer, in the document's own order, so the same
 * selection always implies the same layer.
 */
export function layerForSelection(
  active: number | null,
  selection: readonly number[],
  layers: readonly LayerContents[],
): number | null {
  if (selection.length === 0) return null;
  const chosen = new Set(selection);
  let holding: number | null = null;
  for (const layer of layers) {
    if (!layer.objects.some((object) => chosen.has(object.id))) continue;
    if (layer.id === active) return null;
    if (holding === null) holding = layer.id;
  }
  return holding;
}
