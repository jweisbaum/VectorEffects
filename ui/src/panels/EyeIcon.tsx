/**
 * The layer panel's visibility toggle.
 *
 * Inline SVG for the same reason the tool palette's icons are (invariant 5:
 * no icon font, no sprite fetched at runtime). Stroked in `currentColor` so
 * the button's active colour reaches it. An open eye for a shown layer, a
 * struck-through one for a hidden layer — the toggle has to read as
 * "visibility" at a glance, which two ring glyphs did not.
 */
export function EyeIcon({ open }: { open: boolean }) {
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
      <path d="M2.5 12c2.4-4 5.6-6 9.5-6s7.1 2 9.5 6c-2.4 4-5.6 6-9.5 6s-7.1-2-9.5-6z" />
      <circle cx="12" cy="12" r="2.6" />
      {!open && <path d="M4 20 20 4" />}
    </svg>
  );
}
