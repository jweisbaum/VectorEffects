/**
 * The near-real-time import's button (spec 4.10, M89).
 *
 * A dish receiving, because what the button fetches is what was observed in
 * the last few days, up to now — the calendar beside it names a span of the
 * past, and this one names the present. Inline SVG for the same reason every
 * other icon here is (invariant 5: no icon font, nothing fetched at runtime),
 * stroked in `currentColor`.
 */
export function SatelliteDishIcon() {
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
      {/* The bowl: half a disc, open toward the upper right. */}
      <path d="M4.5 10.5l9 9a6.36 6.36 0 0 1-9-9z" />
      {/* The feed arm, and the feed at its end. */}
      <path d="M9 15l3.5-3.5" />
      <circle cx="13.2" cy="10.8" r="1" fill="currentColor" stroke="none" />
      {/* What it is receiving. */}
      <path d="M14.5 6a4 4 0 0 1 4 4M14.5 2.5a7.5 7.5 0 0 1 7.5 7.5" />
    </svg>
  );
}
