/**
 * The history import's button (spec 4.10, M38).
 *
 * A calendar, because what the button asks for is a date range — the two
 * buttons beside it name a file, and this one names a span of past hours.
 * Inline SVG for the same reason every other icon here is (invariant 5: no
 * icon font, nothing fetched at runtime), stroked in `currentColor`.
 */
export function CalendarIcon() {
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
      <rect x="3.5" y="5" width="17" height="15" rx="2" />
      <path d="M3.5 10h17M8 3.5v3M16 3.5v3" />
      <circle cx="8.5" cy="14.5" r="1" fill="currentColor" stroke="none" />
      <circle cx="12" cy="14.5" r="1" fill="currentColor" stroke="none" />
      <circle cx="15.5" cy="14.5" r="1" fill="currentColor" stroke="none" />
    </svg>
  );
}
