// @vitest-environment happy-dom
import { act, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { PlaybackMap } from "./preparation";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const backend = vi.hoisted(() => ({
  tree: vi.fn(async () => ({ layers: [] as import("../generated/LayerNode").LayerNode[] })),
  tracks: vi.fn(), setKey: vi.fn(), setRange: vi.fn(),
  renderAhead: vi.fn(async () => {}), readiness: vi.fn(async () => ({
  revision: 7, steps: Array.from({ length: 10 }, (_, step) => ({ step, ready: 1, total: 1 })),
})) }));
vi.mock("../ipc", () => ({ api: {
  renderAhead: backend.renderAhead, frameReadiness: backend.readiness,
  documentTree: backend.tree, objectTracks: backend.tracks, setKeyframe: backend.setKey, setActiveRange: backend.setRange,
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


it("toggles shape editing per object and uses the shape track to add keys", async () => {
  const project = { revision: 8, step_count: 10, step_hours: 1, start_unix_s: null } as ProjectSummary;
  backend.tree.mockResolvedValue({ layers: [{ id: 1, name: "Layer", visible: true, locked: false,
    source: "painted", parameter: "wind", grib: null, image: null, gis: null,
    objects: [{ id: 2, name: "Front", tool: "shape_fill", tool_label: "Shape", active_here: true, start_step: 0, end_step: 9 }],
  }] });
  backend.tracks.mockResolvedValue({ object: 2, name: "Front", start_step: 0, end_step: 9,
    tracks: [{ property: "shape", label: "Shape", base: { kind: "bool", value: true }, keys: [],
      interpolations: [{kind:"linear"}], keyed_here: false, interpolated_here: false,
      motion_available: false, motion: false, can_follow: false, follows: null, follows_name: null, inherited: [],
    }],
  });
  backend.setKey.mockResolvedValue(project);
  const changes: (number | null)[] = [];
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  function Host() {
    const [editing, setEditing] = useState<number | null>(null);
    return <Timeline project={project} step={3} onStepChange={() => {}} selection={[]} onSelect={() => {}}
      viewport={[]} playback={{ prepare: () => ({ready:0,total:0,streaming:false}), present: () => false }}
      autoKey={false} onAutoKey={() => {}} onChanged={() => {}} onFramesSelected={() => {}} onKeysSelected={() => {}}
      settings={null} capture={null} onCapture={() => {}} shapeEditing={editing} onShapeEditing={(id) => {
        const next = editing === id ? null : id; changes.push(next); setEditing(next);
      }} />;
  }
  try {
    await act(async () => root.render(<Host />));
    const button = container.querySelector<HTMLButtonElement>('[aria-label="Animate shape of Front"]')!;
    expect(button.getAttribute("aria-pressed")).toBe("false");
    await act(async () => button.click());
    expect(button.getAttribute("aria-pressed")).toBe("true");
    expect(container.querySelector(".tl-track-name")?.textContent).toBe("Shape");
    await act(async () => container.querySelector<HTMLButtonElement>(".tl-key-here")!.click());
    expect(backend.setKey).toHaveBeenCalledWith(2,"shape",3);
    const grid = container.querySelector<HTMLElement>(".tl-track .tl-grid")!;
    const frameWidth = Number.parseFloat(grid.style.width) / project.step_count;
    await act(async () => grid.dispatchEvent(
      new MouseEvent("dblclick", { bubbles: true, clientX: 200 + 6.5 * frameWidth }),
    ));
    expect(backend.setKey).toHaveBeenCalledWith(2,"shape",6);
    await act(async () => button.click());
    expect(button.getAttribute("aria-pressed")).toBe("false");
    expect(changes).toEqual([2,null]);
  } finally {
    await act(async () => root.unmount()); container.remove(); backend.tree.mockResolvedValue({ layers: [] });
  }
});

it("holds the latest released span through the write and delayed tree refresh", async () => {
  const project = { revision: 8, step_count: 10, step_hours: 1, start_unix_s: null } as ProjectSummary;
  const layer = { id: 1, name: "Layer", visible: true, locked: false, source: "painted", parameter: "wind", grib: null, image: null, gis: null,
    objects: [{ id: 2, name: "Front", tool: "shape_fill", tool_label: "Shape", active_here: true, start_step: 0, end_step: 9 }] };
  backend.tree.mockResolvedValue({ layers: [layer] });
  let written!: (project: ProjectSummary) => void;
  let refreshed!: (tree: { layers: typeof layer[] }) => void;
  backend.setRange.mockReturnValue(new Promise(resolve => { written = resolve; }));
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  const changed = vi.fn();
  try {
    await act(async () => root.render(<Timeline project={project} step={0} onStepChange={() => {}} selection={[]} onSelect={() => {}}
      viewport={[]} playback={{ prepare: () => ({ready:0,total:0,streaming:false}), present: () => false }}
      autoKey={false} onAutoKey={() => {}} onChanged={changed} onFramesSelected={() => {}} onKeysSelected={() => {}}
      settings={null} capture={null} onCapture={() => {}} />));
    const bar = container.querySelector<HTMLElement>(".tl-range")!;
    const frameWidth = Number.parseFloat(bar.style.width) / 10;
    const grip = container.querySelector<HTMLElement>('[aria-label="End frame for Front"]')!;
    await act(async () => grip.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0, pointerId: 1 })));
    await act(async () => {
      grip.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX: 200 + 5.2 * frameWidth, pointerId: 1 }));
      grip.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1 }));
    });
    expect(backend.setRange).toHaveBeenCalledWith(2, 0, 5);
    expect(bar.style.width).toBe(`${6 * frameWidth}px`);
    backend.tree.mockReturnValueOnce(new Promise(resolve => { refreshed = resolve; }));
    await act(async () => written({ ...project, revision: 9 }));
    expect(bar.style.width).toBe(`${6 * frameWidth}px`);
    expect(changed).not.toHaveBeenCalled();
    await act(async () => refreshed({ layers: [{ ...layer, objects: [{ ...layer.objects[0]!, end_step: 5 }] }] }));
    expect(bar.style.width).toBe(`${6 * frameWidth}px`);
    expect(changed).toHaveBeenCalledTimes(1);
  } finally { await act(async () => root.unmount()); container.remove(); backend.tree.mockResolvedValue({ layers: [] }); }
});

it("scrubs the ruler without starting text selection and stops on cancel", async () => {
  backend.tree.mockResolvedValue({layers: []});
  const project = {revision: 19, step_count:10, step_hours:1, start_unix_s:null} as ProjectSummary;
  const move = vi.fn();
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Timeline project={project} step={0} onStepChange={move} selection={[]} onSelect={() => {}}
      viewport={[]} playback={{prepare: () => ({ready:0,total:0,streaming:false}), present: () => false}} autoKey={false} onAutoKey={() => {}} onChanged={() => {}} onFramesSelected={() => {}}
      onKeysSelected={() => {}} settings={null} capture={null} onCapture={() => {}} />));
    const ruler = container.querySelector<HTMLElement>(".tl-ruler .tl-grid")!;
    const press = new PointerEvent("pointerdown", {bubbles:true, cancelable:true, button:0, clientX:240});
    await act(async () => ruler.dispatchEvent(press));
    expect(press.defaultPrevented).toBe(true);
    await act(async () => window.dispatchEvent(new PointerEvent("pointermove", {clientX:310})));
    expect(move).toHaveBeenCalledTimes(2);
    await act(async () => window.dispatchEvent(new PointerEvent("pointercancel")));
    await act(async () => window.dispatchEvent(new PointerEvent("pointermove", {clientX:360})));
    expect(move).toHaveBeenCalledTimes(2);
  } finally { await act(async () => root.unmount()); container.remove(); }
});

it.each(["macro", "patch"])("does not offer shape animation for a %s", async (tool) => {
  backend.tree.mockResolvedValue({ layers: [{id:1, name:"Layer", visible:true, locked:false, source:"painted", parameter:"wind", grib:null, image:null, gis:null,
    objects:[{id:2, name:"Capture", tool, tool_label:tool, active_here:true, start_step:0, end_step:9}]}]});
  const project = {revision: 20, step_count:10, step_hours:1, start_unix_s:null} as ProjectSummary;
  const container = document.createElement("div"); document.body.append(container); const root=createRoot(container);
  try {
    await act(async () => root.render(<Timeline project={project} step={0} onStepChange={() => {}} selection={[]} onSelect={() => {}}
      viewport={[]} playback={{prepare: () => ({ready:0,total:0,streaming:false}), present: () => false}} autoKey={false} onAutoKey={() => {}} onChanged={() => {}} onFramesSelected={() => {}}
      onKeysSelected={() => {}} settings={null} capture={null} onCapture={() => {}} />));
    expect(container.querySelector(".tl-object-name")?.textContent).toBe("Capture");
    expect(container.querySelector(".tl-shape-toggle")).toBeNull();
  } finally { await act(async () => root.unmount()); container.remove(); backend.tree.mockResolvedValue({layers:[]}); }
});
