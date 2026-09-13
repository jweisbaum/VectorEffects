import { expect, it } from "vitest";
import { preparationTargets, type PreparationRequest } from "./preparation";

const run: PreparationRequest = { step: 5, first: 0, last: 23, loop: true, rate: 8, states: [] };

it("prepares a fitting run completely in playback order, even while paused", () => {
  const plan = preparationTargets(run, 128, 26 * 128, 100);
  expect(plan.streaming).toBe(false);
  expect(plan.targets).toHaveLength(24);
  expect(plan.targets.slice(0, 3)).toEqual([5, 6, 7]);
  expect(new Set(plan.targets).size).toBe(24);
});

it("bounds a rolling window and grows lookahead for higher rates/latency", () => {
  const slow = preparationTargets({ ...run, last: 239 }, 128, 4096, 100);
  const fast = preparationTargets({ ...run, last: 239, rate: 30 }, 128, 4096, 800);
  expect(slow.streaming).toBe(true);
  expect(fast.targets.length).toBeGreaterThan(slow.targets.length);
  expect((fast.targets.length + 2) * 128).toBeLessThanOrEqual(4096);
});

it("keeps a macro's boundaries and does not wrap a streaming non-looping run", () => {
  const macro = preparationTargets({ ...run, step: 9, first: 4, last: 9 }, 1, 100, 100);
  expect(macro.targets).toEqual([9, 4, 5, 6, 7, 8]);
  const end = preparationTargets({ ...run, step: 238, last: 239, loop: false }, 192, 768, 100);
  expect(end.targets).toEqual([238, 239]);
});
