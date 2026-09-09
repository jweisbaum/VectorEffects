/**
 * The lifetime window's drag: where it lands, and when the preview stands down.
 */
import { describe, expect, it } from "vitest";

import {
  commitRangeDrag,
  committedRange,
  draggedRange,
  type RangeGrab,
  unchanged,
} from "./rangeDrag";

const grab = (over: Partial<RangeGrab> = {}): RangeGrab => ({
  object: 7,
  grip: "whole",
  other: 4,
  offset: 0,
  from: [0, 4],
  ...over,
});

describe("dragging a lifetime window", () => {
  /** An end follows the pointer, and may cross the other end. */
  it("lets an end grip pass its partner and sorts it out on release", () => {
    const held = grab({ grip: "start", other: 6 });
    expect(draggedRange(held, 9, 23).step).toBe(9);
    expect(committedRange(held, draggedRange(held, 9, 23))).toEqual([6, 9]);
    expect(committedRange(held, draggedRange(held, 2, 23))).toEqual([2, 6]);
  });

  /**
   * The whole window keeps its length. Checked as that property rather than
   * against the arithmetic: whatever the pointer does, the window is exactly as
   * long as it was.
   */
  it("keeps a whole window's length wherever it is dragged", () => {
    const held = grab({ other: 4, offset: 2 });
    for (const at of [-30, 0, 3, 11, 23, 200]) {
      const [start, end] = committedRange(held, draggedRange(held, at, 23));
      expect(end - start, `at ${at}`).toBe(4);
      expect(start, `at ${at}`).toBeGreaterThanOrEqual(0);
      expect(end, `at ${at}`).toBeLessThanOrEqual(23);
    }
  });

  /**
   * And it holds the step it was grabbed by under the pointer, so the bar does
   * not leap sideways the moment it is picked up.
   */
  it("keeps the grabbed step under the pointer", () => {
    const held = grab({ other: 4, offset: 3 });
    expect(draggedRange(held, 12, 23).step).toBe(9);
    expect(draggedRange(held, 5, 23).step).toBe(2);
  });

  /**
   * A nudge that stays inside the step it began in asks for the window it
   * already had, and writing that would put an entry in the history that
   * undoes nothing.
   */
  it("knows when a drag is asking for the window it already had", () => {
    const held = grab({ other: 4, offset: 2, from: [6, 10] });
    expect(unchanged(held, draggedRange(held, 8, 23))).toBe(true);
    expect(unchanged(held, draggedRange(held, 9, 23))).toBe(false);

    const end = grab({ grip: "end", other: 6, from: [6, 12] });
    expect(unchanged(end, draggedRange(end, 12, 23))).toBe(true);
    expect(unchanged(end, draggedRange(end, 13, 23))).toBe(false);
  });

  /** A window as long as the timeline has nowhere to go. */
  it("has nowhere to slide a window that fills the timeline", () => {
    const held = grab({ other: 23, offset: 5 });
    expect(draggedRange(held, 0, 23).step).toBe(0);
    expect(draggedRange(held, 23, 23).step).toBe(0);
  });
});

describe("committing a drag", () => {
  /**
   * The regression (M62): the preview was cleared first and the backend asked
   * second, so the bar was drawn from the document's old numbers for the length
   * of the round trip — it snapped back and then jumped forward again.
   */
  it("holds the preview until the write has landed", async () => {
    const order: string[] = [];
    let release: (value: string) => void = () => {};
    const write = () =>
      new Promise<string>((resolve) => {
        order.push("asked");
        release = resolve;
      });

    const done = commitRangeDrag(
      write,
      () => order.push("applied"),
      () => order.push("failed"),
      () => order.push("cleared"),
    );
    expect(order).toEqual(["asked"]);
    release("ok");
    await done;
    expect(order).toEqual(["asked", "applied", "cleared"]);
  });

  /** And stands down on a refusal too, or the bar would be stuck mid-drag. */
  it("clears the preview when the write is refused", async () => {
    const order: string[] = [];
    await commitRangeDrag(
      () => Promise.reject(new Error("locked")),
      () => order.push("applied"),
      () => order.push("failed"),
      () => order.push("cleared"),
    );
    expect(order).toEqual(["failed", "cleared"]);
  });
});
