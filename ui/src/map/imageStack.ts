/**
 * Where an image layer sits relative to the field (spec.md 4.9, M49).
 *
 * An image is a reference to trace against, so it is drawn under the field.
 * But a layer moved above every field layer is one the user has asked to see,
 * and the field is nearly opaque — leaving it underneath makes it vanish,
 * which is what moving an imported chart to the top of the stack did.
 *
 * Its own module because the rule is about *order*, and an order is the kind
 * of thing that is wrong by one and looks nearly right.
 */

/** As much of a layer as this rule cares about. */
export interface StackedLayer {
  id: number;
  visible: boolean;
  /** Whether the layer is a picture rather than a field. */
  isImage: boolean;
}

/**
 * The image layers that draw over the field: those above every visible layer
 * that carries one.
 *
 * `layers` is bottom-first, as the document stores it. Hidden layers are not
 * on the map at all and count for neither side: a hidden field layer does not
 * hold an image down, and a hidden image is not drawn wherever it sits.
 */
export function imagesOverField(layers: readonly StackedLayer[]): Set<number> {
  const shown = layers.filter((layer) => layer.visible);
  const lastField = shown.reduce((at, layer, index) => (layer.isImage ? at : index), -1);
  return new Set(
    shown.filter((layer, index) => layer.isImage && index > lastField).map((layer) => layer.id),
  );
}
