/**
 * The lifetime window's drag: when the preview stands down.
 */
import { describe, expect, it } from "vitest";

import { commitRangeDrag } from "./rangeDrag";

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
