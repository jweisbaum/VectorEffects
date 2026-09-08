/**
 * Choosing a colour gradient (spec.md 5.3, M42).
 *
 * A dropdown, because the settings are a column of them and a list of nine
 * open at once would push everything else off the panel. Built rather than
 * taken from the platform because a native `select` renders nothing but text
 * in its options, and the one thing worth knowing about a gradient is what it
 * looks like: the closed control shows the chosen run of colours, and opening
 * it shows all of them.
 *
 * It is a `listbox` in the ARIA sense and behaves like one for the things a
 * keyboard user will try — Escape closes it, Enter and Space open it, and the
 * arrows move through the options — so it is reachable without a pointer.
 */
import { useEffect, useId, useRef, useState } from "react";

import { rampStops } from "../map/ramp";

/** One gradient on offer. */
export interface PickableGradient {
  id: string;
  label: string;
  note: string;
  stops: readonly (readonly number[])[];
  /**
   * Whether this build has the gradient. A project written by a later
   * version names one that is not here; it is shown, because that is what
   * the file says, and cannot be chosen, because there is nothing to choose.
   */
  known: boolean;
}

/** The run of colours a gradient names, as a CSS gradient. */
function barOf(gradient: PickableGradient): string {
  if (!gradient.known || gradient.stops.length === 0) return "transparent";
  const stops = rampStops(gradient.stops as readonly (readonly [number, number, number])[]);
  return `linear-gradient(to right, ${stops.join(", ")})`;
}

export default function GradientPicker({
  label,
  value,
  gradients,
  onChoose,
}: {
  /** What the control is for, e.g. "…colour gradient for wind". */
  label: string;
  /** The identifier the project holds. */
  value: string;
  /** Everything on offer, in the order it is listed. */
  gradients: readonly PickableGradient[];
  onChoose: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const held = useRef<HTMLDivElement | null>(null);
  const listId = useId();
  const chosen = gradients.find((entry) => entry.id === value);

  // Closed by a press anywhere else and by Escape. Both are listened for on
  // the document rather than on the control: a press that lands on the map
  // behind the dialog has to close it too, and it will never reach the
  // control's own handlers.
  useEffect(() => {
    if (!open) return undefined;
    const away = (event: PointerEvent) => {
      if (!held.current?.contains(event.target as Node)) setOpen(false);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      document.removeEventListener("keydown", key);
    };
  }, [open]);

  const step = (by: number) => {
    const pickable = gradients.filter((entry) => entry.known);
    if (pickable.length === 0) return;
    const at = pickable.findIndex((entry) => entry.id === value);
    const next = pickable[(at + by + pickable.length) % pickable.length];
    if (next) onChoose(next.id);
  };

  return (
    <div className="gradient-picker" ref={held}>
      <span className="gradient-picker-label">{label}</span>
      <div className="gradient-picker-control">
        <button
          type="button"
          className="gradient-trigger"
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={open ? listId : undefined}
          title={chosen?.note}
          onClick={() => setOpen((was) => !was)}
          onKeyDown={(event) => {
            // The arrows move the choice whether or not the list is open,
            // which is how a native picker behaves and is the fastest way to
            // see what each gradient does to the map behind the dialog.
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              step(event.key === "ArrowDown" ? 1 : -1);
            }
          }}
        >
          <span className="gradient-bar" aria-hidden="true" style={{ background: chosen ? barOf(chosen) : "transparent" }} />
          <span className="gradient-name">{chosen?.label ?? value}</span>
          <span className="gradient-caret" aria-hidden="true">
            ▾
          </span>
        </button>
        {open && (
          <ul className="gradient-list" role="listbox" id={listId} aria-label={label}>
            {gradients.map((entry) => (
              <li
                key={entry.id}
                role="option"
                aria-selected={entry.id === value}
                aria-disabled={!entry.known}
                className={entry.id === value ? "gradient-option on" : "gradient-option"}
                title={entry.note}
                onClick={() => {
                  setOpen(false);
                  if (entry.known && entry.id !== value) onChoose(entry.id);
                }}
              >
                <span className="gradient-bar" aria-hidden="true" style={{ background: barOf(entry) }} />
                <span className="gradient-name">{entry.label}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
