import { expect, it } from "vitest";
import { ReadinessPoller } from "./readiness";

it("coalesces overlapping refreshes and ignores a disposed viewport", async () => {
  const completions: Array<(value: number) => void> = [];
  const accepted: number[] = [];
  const poller = new ReadinessPoller(() => new Promise<number>((resolve) => completions.push(resolve)), (value) => accepted.push(value));
  for (let i = 0; i < 100; i++) poller.request();
  expect(completions).toHaveLength(1);
  completions[0]!(1);
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(accepted).toEqual([1]);
  expect(completions).toHaveLength(2);
  poller.dispose();
  completions[1]!(2);
  await new Promise((resolve) => setTimeout(resolve, 0));
  poller.request();
  expect(accepted).toEqual([1]);
  expect(completions).toHaveLength(2);
});

it("can refresh after an error instead of remaining stuck in flight", async () => {
  let attempts = 0;
  const accepted: number[] = [];
  const poller = new ReadinessPoller(async () => {
    if (++attempts === 1) throw new Error("temporary failure");
    return attempts;
  }, (value) => accepted.push(value));
  poller.request();
  poller.request();
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(accepted).toEqual([2]);
  poller.dispose();
});
