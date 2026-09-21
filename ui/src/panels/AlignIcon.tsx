/**
 * The image layer's "align by pointing" button.
 *
 * Inline SVG for the same reason every other icon here is (invariant 5: no
 * icon font, no sprite fetched at runtime), stroked in `currentColor` so the
 * button's own colour reaches it.
 *
 * A crosshair tied by a line to a mark: the gesture is *this* place in the
 * picture is *that* place on the map, twice per pair, and the glyph says the
 * pairing rather than the picking. A lone crosshair would read as "pick a
 * point", which is only half of it.
 */
export function AlignIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width={14}
      height={14}
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {/* The place picked in the picture. */}
      <circle cx="8" cy="8" r="3.1" />
      <path d="M8 2.2V4.3M8 11.7v2.1M2.2 8h2.1M11.7 8h2.1" />
      {/* Tied to where it belongs. */}
      <path d="M11.4 13.1 15.6 17.3" strokeDasharray="1.6 1.9" />
      <circle cx="18.2" cy="18.2" r="1.7" fill="currentColor" stroke="none" />
    </svg>
  );
}
