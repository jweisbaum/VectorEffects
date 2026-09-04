/**
 * A number input that can be cleared.
 *
 * Every numeric field in the app goes through this. What it adds over a plain
 * controlled `<input type="number">` is a *draft*: while the field is being
 * edited it shows what was typed, not what is committed, so an empty box stays
 * empty and a value that is still being entered is not overwritten by the one
 * it is replacing. See `numberEntry.ts` for the rule and why it exists.
 *
 * Values are committed as they are typed — a field that only committed on blur
 * would break the live preview every option bar depends on — but they are
 * *clamped* only on the way out, since a bound applied mid-entry is the same
 * trap in another costume: a minimum of 1 turns "40" into "14".
 */

import { useState } from "react";

import { clamped, typedValue } from "./numberEntry";

export default function NumberField({
  value,
  onCommit,
  min = null,
  max = null,
  step,
  title,
  format = (v) => String(Math.round(v * 1e6) / 1e6),
  commitWhileTyping = true,
}: {
  /** The committed value, in the units the field shows. */
  value: number;
  onCommit: (value: number) => void;
  min?: number | null;
  max?: number | null;
  step?: number | string;
  title?: string;
  /** How a committed value is written into the box when it is not being edited. */
  format?: (value: number) => string;
  /**
   * Whether each keystroke commits.
   *
   * True for nearly everything: the map previews what the bar holds. False for
   * a field whose commit is expensive or gated — the step count, which asks for
   * a confirmation before it deletes keyframes, and must not ask on the way
   * from 1 to 12 (spec.md 4.1).
   */
  commitWhileTyping?: boolean;
}) {
  const [draft, setDraft] = useState<string | null>(null);

  const finish = () => {
    const typed = draft === null ? null : typedValue(draft);
    setDraft(null);
    if (typed === null) return;
    // Only a change is committed: with `commitWhileTyping` the typed value is
    // already `value` by now, so this fires exactly when the clamp moved it.
    const inside = clamped(typed, min, max);
    if (inside !== value) onCommit(inside);
  };

  return (
    <input
      type="number"
      min={min ?? undefined}
      max={max ?? undefined}
      step={step}
      title={title}
      value={draft ?? format(value)}
      onChange={(event) => {
        const raw = event.target.value;
        setDraft(raw);
        if (!commitWhileTyping) return;
        const typed = typedValue(raw);
        if (typed !== null) onCommit(typed);
      }}
      onBlur={finish}
      onKeyDown={(event) => {
        // Enter is a commit for a field that does not commit as it is typed,
        // and harmless for one that does.
        if (event.key === "Enter") finish();
      }}
    />
  );
}
