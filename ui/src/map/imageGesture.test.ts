import { expect, it, vi } from "vitest";
import { imageGesture } from "./imageGesture";

it("coalesces pointer reports, writes the release, then ends before the next drag", async () => {
  const calls: string[] = [];
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => { release = resolve; });
  const error = vi.fn();
  const first = imageGesture(Promise.resolve(), async (point: number) => {
    calls.push(`first:${point}`);
    if (point === 1) await blocked;
  }, async () => { calls.push("end:first"); }, error);
  first.push(1);
  await Promise.resolve();
  first.push(2); first.push(3); first.finish();
  const second = imageGesture(first.done, async (point: number) => { calls.push(`second:${point}`); },
    async () => { calls.push("end:second"); }, error);
  second.push(4); second.finish();
  expect(calls).toEqual(["first:1"]);
  release();
  await second.done;
  expect(calls).toEqual(["first:1", "first:3", "end:first", "second:4", "end:second"]);
  expect(error).not.toHaveBeenCalled();
});

it("ends after a rejected write and lets the next drag proceed", async () => {
  const error = vi.fn();
  const end = vi.fn(async () => {});
  const first = imageGesture(Promise.resolve(), async () => { throw new Error("refused"); }, end, error);
  first.push(1); first.finish();
  await first.done;
  expect(error).toHaveBeenCalledTimes(1);
  expect(end).toHaveBeenCalledTimes(1);
});
