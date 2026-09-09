/**
 * Which tools a layer will take (spec.md 6.1, M51).
 *
 * The backend has always refused the wrong ones — `creation_layer` turns away
 * a field object aimed at an imported layer, and the eraser turns away an
 * image — but the refusal arrived on release, after the stroke had been drawn
 * and previewed. The cursor says it on hover instead, so the answer comes
 * before the gesture rather than after it.
 *
 * The rule is the backend's, restated: this is the frontend half of one
 * decision, not a second decision. A tool refused here and accepted there
 * would be a stroke that cannot be drawn; accepted here and refused there is
 * what this exists to stop.
 */

/** What a layer holds, as `LayerNode.source` spells it. */
export type LayerSourceName = "painted" | "raster" | "zarr" | "image";

/**
 * What a tool does to a layer.
 *
 * - `adds` paints a field of its own: the brush, the shapes, the curve, a
 *   macro. `Placing::Field` on the Rust side.
 * - `edits` changes what is already there: the mask, the clone stamp, the
 *   modifiers, and the eraser, which is no object at all. `Placing::Edit`.
 * - `neither` touches no field: the hand, the selection, a measurement, a
 *   capture.
 */
export type ToolKindOfWork = "adds" | "edits" | "neither";

/**
 * Whether a layer will take that kind of work.
 *
 * A **painted** layer takes everything. An **imported** field — a forecast or
 * a fetched history — takes edits but not a field of its own: it has one
 * already, read from a file, and a second painted over it would be neither
 * the file's nor the user's. An **image** takes nothing at all: it reaches no
 * scene and no export, so there is no field there to add to or to change.
 */
export function layerTakes(source: LayerSourceName, work: ToolKindOfWork): boolean {
  if (work === "neither") return true;
  switch (source) {
    case "painted":
      return true;
    case "raster":
    case "zarr":
      return work === "edits";
    case "image":
      return false;
  }
}

/**
 * Which layer a gesture aimed at the map would land on (M68).
 *
 * `null` means nothing is chosen, which is the top of the stack — the rule
 * `document::creation_layer` follows — so the two cannot come to disagree
 * about where a stroke goes. The list is bottom-first, as the document is, so
 * the top is the last of it.
 */
export function targetLayer(
  active: number | null,
  layers: ReadonlyArray<{ id: number }>,
): number | null {
  if (active !== null) return active;
  return layers[layers.length - 1]?.id ?? null;
}

/**
 * Whether a gesture aimed at the map would land somewhere off it (M68).
 *
 * A hidden layer is not on the map, so a stroke over it was aimed at whatever
 * *is* — and while the pointer is down the map shows the edit happening to the
 * layers that are visible, because a preview drawn over the composite has no
 * layer of its own. The liquify is the plainest case: its preview displaces
 * the rendered field itself.
 *
 * **A layer nobody has heard of counts as visible.** The tree arrives a moment
 * after the project does, and refusing every gesture in that window would be a
 * worse lie than allowing one — the backend refuses it on release either way.
 */
export function aimedOffTheMap(
  active: number | null,
  layers: ReadonlyArray<{ id: number; visible: boolean }>,
): boolean {
  const target = targetLayer(active, layers);
  if (target === null) return false;
  return layers.some((layer) => layer.id === target && !layer.visible);
}
