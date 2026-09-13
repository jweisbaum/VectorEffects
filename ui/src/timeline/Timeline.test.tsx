// @vitest-environment happy-dom
import { act, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { PlaybackMap } from "./preparation";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const backend = vi.hoisted(() => ({ renderAhead: vi.fn(async () => {}), readiness: vi.fn(async () => ({
  revision: 7, steps: Array.from({ length: 10 }, (_, step) => ({ step, ready: 1, total: 1 })),
})) }));
vi.mock("../ipc", () => ({ api: {
  renderAhead: backend.renderAhead, frameReadiness: backend.readiness,
  documentTree: async () => ({ layers: [] }),
} }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
const Timeline = (await import("./Timeline")).default;

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });

it("prepares while paused, publishes only drawn steps, and maintains the selected rate", async () => {
  vi.useFakeTimers();
  let now = 0;
  let id = 0;
  const callbacks = new Map<number, FrameRequestCallback>();
  vi.spyOn(performance, "now").mockImplementation(() => now);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callbacks.set(++id, callback); return id; });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => callbacks.delete(id));
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const drawn: number[] = [];
  const published: number[] = [];
  const playback: PlaybackMap = {
    prepare: vi.fn(() => ({ ready: 10, total: 10, streaming: false })),
    present: (step) => { if (now < 500) return false; drawn.push(step); return true; },
  };
  const project = { revision: 7, step_count: 10, step_hours: 1, start_unix_s: null } as ProjectSummary;
  function Host() {
    const [step, setStep] = useState(0);
    const move = useCallback((step: number) => {
      expect(drawn.at(-1)).toBe(step);
      published.push(step);
      setStep(step);
    }, []);
    return <Timeline project={project} step={step} onStepChange={move} selection={[]} onSelect={() => {}}
      viewport={[{ z: 0, x: 0, y: 0 }]} playback={playback} autoKey={false} onAutoKey={() => {}}
      onChanged={() => {}} onFramesSelected={() => {}} onKeysSelected={() => {}} settings={null}
      capture={null} onCapture={() => {}} />;
  }
  try {
    await act(async () => root.render(<Host />));
    expect(playback.prepare).toHaveBeenCalled();
    expect(published).toEqual([]);
    await act(async () => {
      container.querySelector<HTMLButtonElement>('[aria-label="Loop"]')!.click();
      container.querySelector<HTMLButtonElement>(".tl-transport button")!.click();
    });
    for (let frame = 1; frame <= 330; frame++) {
      now = frame * 1000 / 60;
      await act(async () => {
        const pending = [...callbacks.values()];
        callbacks.clear();
        pending.forEach((callback) => callback(now));
      });
      if (now < 500) expect(published).toEqual([]);
    }
    // One at buffer recovery, then 40 over the following five seconds.
    expect(published).toHaveLength(41);
    expect(published.every((step, index) => step === (index + 1) % 10)).toBe(true);
    expect(backend.renderAhead.mock.calls.length).toBeLessThanOrEqual(2);
    expect(backend.readiness).toHaveBeenCalledTimes(1);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
