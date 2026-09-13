import { useState } from "react";

import type { GribLayerInfo } from "../generated/GribLayerInfo";
import NumberField from "../NumberField";
import { useUnits } from "../settings/units";

/** A band in whole display units, low end first — the slider's own resolution. */
type Band = [number, number];

const same = (a: Band, b: Band) => a[0] === b[0] && a[1] === b[1];

/**
 * A band written at release, until the document is seen holding it.
 *
 * `revision` is the document revision the write returned, or null while
 * the write is still in flight.
 */
type Pending = { band: Band; revision: number | null };

/**
 * The band of speeds an imported field keeps (spec.md 4.8).
 *
 * A forecast is easier to read one band at a time: the calms, the gale, the
 * jet. A sample outside the band is dropped exactly as a missing one is, so
 * what is beneath shows through — including the layer's own painted objects,
 * which is what makes this a filter on the *import* and not on the layer.
 *
 * Two sliders and two fields, not a dual-thumb control: the two ends are two
 * numbers, and they are often typed rather than dragged. **A dragged thumb
 * stops at the other one.** Each slider is a controlled input, so a thumb
 * pushed past the other cannot go there; what the browser did instead was pin
 * the thumb under the hand and move the *other* one to the pointer, which
 * looked like the two sliders being tied together. A typed value may still
 * cross, and the backend orders it.
 *
 * **The control works in whole display units.** The document holds the band as f32
 * metres per second, and 7 kt comes back from it as 7.00000017 kt: not the
 * integer the slider sent, not equal to it, and not a value a step-1 slider
 * can hold. The band read from the document is rounded to the step before
 * anything compares or displays it, or a release that moved nothing is a
 * write, and a clamped thumb sits on a value the browser then snaps away from.
 */
export default function SpeedFilter({
  grib,
  treeRevision,
  onChange,
}: {
  grib: Pick<GribLayerInfo, "speed_min_mps" | "speed_max_mps" | "speed_ceiling_mps">;
  /**
   * The document revision `grib` was read at.
   *
   * The band in `grib` is one round trip behind the document: the summary a
   * write returns carries the new revision, and the tree that carries the new
   * band is fetched after it. This is how the component knows the tree has
   * caught up with a write it made.
   */
  treeRevision: number;
  /**
   * One write per release, per typed value, or per checkbox: `gesture` is
   * always null now and stays in the signature for the command, which still
   * coalesces under a key for any caller that has a reason to. Resolves to
   * the document revision after the write, or null if the write was refused.
   */
  onChange: (
    minMps: number | null,
    maxMps: number | null,
    gesture: string | null,
  ) => Promise<number | null>;
}) {
  const units = useUnits();
  const { speedFromMps, speedToMps } = units;
  const ceiling = Math.max(5, Math.ceil(speedFromMps(grib.speed_ceiling_mps)));
  const on = grib.speed_min_mps !== null && grib.speed_max_mps !== null;
  const stored: Band = [
    on ? Math.round(speedFromMps(grib.speed_min_mps ?? 0)) : 0,
    on ? Math.round(speedFromMps(grib.speed_max_mps ?? 0)) : ceiling,
  ];
  /**
   * The band while the thumb is down.
   *
   * **Nothing is written until the pointer comes up.** Every tick used to be
   * a document write, and a write invalidates every tile of the imported
   * field and re-renders every panel — so the thumb moved a round trip and a
   * viewport of tiles behind the hand. The thumb is local state now, the
   * map and the panels are left alone for the length of the drag, and the
   * release writes the band once: one history entry per release, with no
   * coalescing key because there is nothing to coalesce.
   */
  const [dragging, setDragging] = useState<Band | null>(null);
  /**
   * The band a release wrote, until the document is seen holding it.
   *
   * The write is one round trip and the tree that carries the new band is a
   * second one, after it. A thumb that went back to showing the document at
   * release sat at the old value for both and then jumped to the released
   * one; one that went back when the write settled still sat at the old value
   * for the second. So the released band is shown until the tree fetched at
   * the write's own revision (or a later one) has arrived — or the write was
   * refused, and the old band is the truth again.
   */
  const [pending, setPending] = useState<Pending | null>(null);
  // The tree has caught up with the write: stop holding. Done in render, so
  // no frame shows anything but the document from here on.
  if (pending !== null && pending.revision !== null && treeRevision >= pending.revision) {
    setPending(null);
  }
  const [bandUnit, setBandUnit] = useState(units.speedUnit);
  if (bandUnit !== units.speedUnit) {
    setBandUnit(units.speedUnit);
    setDragging(null);
    setPending(null);
  }
  const [low, high] = dragging ?? pending?.band ?? stored;

  const set = (nextLow: number, nextHigh: number) =>
    onChange(speedToMps(nextLow), speedToMps(nextHigh), null);
  const endDrag = () => {
    const band = dragging;
    if (band === null) return;
    setDragging(null);
    if (same(band, stored)) return;
    setPending({ band, revision: null });
    // A newer release replaces `pending` with its own band; the identity
    // check keeps an older write's answer from touching it.
    set(band[0], band[1]).then(
      (revision) =>
        setPending((current) =>
          current?.band !== band ? current : revision === null ? null : { band, revision },
        ),
      () => setPending((current) => (current?.band === band ? null : current)),
    );
  };

  return (
    <div className="grib-filter">
      <label title="Keep only the speeds inside this band; the rest of the imported field is dropped, and whatever is beneath it shows through.">
        <input
          type="checkbox"
          checked={on}
          onChange={(e) => (e.target.checked ? set(0, ceiling) : onChange(null, null, null))}
        />
        Speed filter
      </label>
      {on && (
        <div className="grib-filter-band">
          <span className="grib-filter-row">
            <input
              type="range"
              min={0}
              max={ceiling}
              step={1}
              value={low}
              onChange={(e) => setDragging([Math.min(Number(e.target.value), high), high])}
              onPointerUp={endDrag}
              onKeyUp={endDrag}
              onBlur={endDrag}
              title="Slowest speed kept"
            />
            <NumberField
              min={0}
              value={low}
              format={(v) => String(Math.round(v))}
              onCommit={(next) => set(next, high)}
            />
          </span>
          <span className="grib-filter-row">
            <input
              type="range"
              min={0}
              max={ceiling}
              step={1}
              value={high}
              onChange={(e) => setDragging([low, Math.max(Number(e.target.value), low)])}
              onPointerUp={endDrag}
              onKeyUp={endDrag}
              onBlur={endDrag}
              title="Fastest speed kept"
            />
            <NumberField
              min={0}
              value={high}
              format={(v) => String(Math.round(v))}
              onCommit={(next) => set(low, next)}
            />
          </span>
          <span className="muted">{units.speedUnit} · speeds outside this range are hidden</span>
        </div>
      )}
    </div>
  );
}
