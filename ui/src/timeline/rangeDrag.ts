/**
 * Dragging an object's lifetime window on the timeline (spec.md 9.4).
 *
 * The maths and the ordering, out of the component so both can be tested: the
 * bug this exists to prevent is a frame of the wrong thing on screen, which no
 * type checker sees and which is over before anyone can point at it.
 */

/** Which part of the window the pointer took hold of. */
export type RangeGrip = "start" | "end" | "whole";

/** A window being dragged, as the bar draws it. */
export interface RangeDrag {
  /** The object whose window it is. */
  object: number;
  grip: RangeGrip;
  /** The step under the pointer, or — for `whole` — the window's new start. */
  step: number;
}

/** What the drag needs to remember from the press. */
export interface RangeGrab {
  object: number;
  grip: RangeGrip;
  /** The end that is not moving; for `whole`, the window's length in steps. */
  other: number;
  /** For `whole`: how far into the window the pointer took hold, in steps. */
  offset: number;
  /** The window as it stood when it was taken hold of, inclusive. */
  from: readonly [number, number];
}

/**
 * Where the window sits while the pointer is at `at`.
 *
 * An end grip follows the pointer and the window may turn inside out, which
 * [`committedRange`] sorts out on release. The **whole** window keeps its
 * length and slides: it holds the step the pointer took hold of under the
 * pointer, so the bar does not jump to centre itself on the press, and it
 * stops at the ends of the timeline rather than sliding off and coming back
 * shorter (M63).
 */
export function draggedRange(grab: RangeGrab, at: number, last: number): RangeDrag {
  if (grab.grip !== "whole") {
    return { object: grab.object, grip: grab.grip, step: at };
  }
  const span = grab.other;
  const start = Math.min(Math.max(at - grab.offset, 0), Math.max(0, last - span));
  return { object: grab.object, grip: "whole", step: start };
}

/** The window a finished drag asks for, as an inclusive `[start, end]`. */
export function committedRange(grab: RangeGrab, drag: RangeDrag): [number, number] {
  if (grab.grip === "whole") return [drag.step, drag.step + grab.other];
  const [a, b] = grab.grip === "start" ? [drag.step, grab.other] : [grab.other, drag.step];
  return [Math.min(a, b), Math.max(a, b)];
}

/**
 * Whether a finished drag is asking for the window it already had.
 *
 * A pointer that moved a few pixels without leaving the step it started in
 * lands back where it began. Writing that would put an entry in the history
 * that undoes nothing — the same guard the property writes carry.
 */
export function unchanged(grab: RangeGrab, drag: RangeDrag): boolean {
  const [start, end] = committedRange(grab, drag);
  return start === grab.from[0] && end === grab.from[1];
}

/**
 * Commits a drag, holding the preview until the document has caught up (M62).
 *
 * The order is the whole point. Clearing the preview first and *then* asking
 * the backend leaves the bar drawn from the document's old numbers for as long
 * as the round trip takes: the window snaps back to where it started and then
 * jumps to where it was dropped, which reads as the drop having failed and
 * been retried. The preview is what the user is looking at, so it stands until
 * there is something newer to draw.
 */
export async function commitRangeDrag<T>(
  write: () => Promise<T>,
  onDone: (value: T) => void,
  onError: (why: unknown) => void,
  clearPreview: () => void,
): Promise<void> {
  try {
    onDone(await write());
  } catch (why) {
    onError(why);
  } finally {
    clearPreview();
  }
}
