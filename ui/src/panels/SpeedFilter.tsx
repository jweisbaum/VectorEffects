import { useState } from "react";

import type { GribLayerInfo } from "../generated/GribLayerInfo";
import NumberField from "../NumberField";
import { knotsFromMps, mpsFromKnots } from "../project/format";

/** A band in knots, low end first. */
type Band = [number, number];

const same = (a: Band, b: Band) => a[0] === b[0] && a[1] === b[1];

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
 */
export default function SpeedFilter({
  grib,
  onChange,
}: {
  grib: GribLayerInfo;
  /**
   * One write per release, per typed value, or per checkbox: `gesture` is
   * always null now and stays in the signature for the command, which still
   * coalesces under a key for any caller that has a reason to. The promise
   * is the write's round trip; the thumb holds its place until it settles.
   */
  onChange: (
    minMps: number | null,
    maxMps: number | null,
    gesture: string | null,
  ) => Promise<unknown>;
}) {
  const ceiling = Math.max(5, Math.ceil(knotsFromMps(grib.speed_ceiling_mps)));
  const on = grib.speed_min_mps !== null && grib.speed_max_mps !== null;
  const stored: Band = [
    on ? knotsFromMps(grib.speed_min_mps ?? 0) : 0,
    on ? knotsFromMps(grib.speed_max_mps ?? 0) : ceiling,
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
   * The band a release wrote, until the document has it.
   *
   * The write is a round trip, and the summary that carries the new band
   * comes back after it. A thumb that went back to showing the document the
   * moment the pointer came up sat at the old value for that round trip and
   * then jumped to the released one. It shows the released band instead
   * until the write settles — by then the document agrees, or the write
   * failed and the old band is the truth again.
   */
  const [pending, setPending] = useState<Band | null>(null);
  const [low, high] = dragging ?? pending ?? stored;

  const set = (nextLow: number, nextHigh: number) =>
    onChange(mpsFromKnots(nextLow), mpsFromKnots(nextHigh), null);
  const endDrag = () => {
    const band = dragging;
    if (band === null) return;
    setDragging(null);
    if (same(band, stored)) return;
    setPending(band);
    // A release that settles after a newer one must not put away the newer
    // band: the identity check is what tells them apart.
    const settle = () => setPending((current) => (current === band ? null : current));
    set(band[0], band[1]).then(settle, settle);
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
              max={ceiling}
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
              max={ceiling}
              value={high}
              format={(v) => String(Math.round(v))}
              onCommit={(next) => set(low, next)}
            />
          </span>
          <span className="muted">kt, of {ceiling} in the file</span>
        </div>
      )}
    </div>
  );
}
