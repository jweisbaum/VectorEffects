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
  align: vi.fn(async () => ({ revision: 23, step_count: 10, step_hours: 3, start_unix_s: 1_790_812_800 })),
  renderAhead: vi.fn(async () => {}), readiness: vi.fn(async () => ({
  revision: 7, steps: Array.from({ length: 10 }, (_, step) => ({ step, ready: 1, total: 1 })),
})) }));
vi.mock("../ipc", () => ({ api: {
  renderAhead: backend.renderAhead, frameReadiness: backend.readiness,
  documentTree: backend.tree, objectTracks: backend.tracks, setKeyframe: backend.setKey, setActiveRange: backend.setRange,
  alignLayer: backend.align,
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
    prepare: vi.fn(() => ({ ready: 2, total: 3, streaming: true })),
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
    expect(container.textContent).toContain("preparing playback 2/3");
    expect(published).toEqual([]);
    await act(async () => {
      container.querySelector<HTMLButtonElement>('[aria-label="Loop"]')!.click();
      container.querySelector<HTMLButtonElement>(".tl-transport button")!.click();
    });
    expect(container.textContent).not.toContain("preparing playback");
    for (let frame = 1; frame <= 330; frame++) {
      now = frame * 1000 / 60;
      await act(async () => {
        const pending = [...callbacks.values()];
        callbacks.clear();
        pending.forEach((callback) => callback(now));
      });
      expect(container.textContent).not.toContain("preparing playback");
      if (now > 150 && now < 500) expect(container.textContent).toContain("buffering");
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
    source: "painted", parameter: "wind", grib: null, image: null, gis: null, sst: null,
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
  const layer = { id: 1, name: "Layer", visible: true, locked: false, source: "painted", parameter: "wind", grib: null, image: null, gis: null, sst: null,
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

it("holds the playhead at a measurement's floor and dims what is before it", async () => {
  // A feature's speed is measured forward in time (spec.md 10): from its
  // first mark the ruler cannot be scrubbed behind that step.
  backend.tree.mockResolvedValue({layers: []});
  const project = {revision: 21, step_count:10, step_hours:1, start_unix_s:null} as ProjectSummary;
  const move = vi.fn();
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  const render = (step: number, floor: number | null) => root.render(<Timeline project={project} step={step} onStepChange={move} selection={[]} onSelect={() => {}}
    viewport={[]} playback={{prepare: () => ({ready:0,total:0,streaming:false}), present: () => false}} autoKey={false} onAutoKey={() => {}} onChanged={() => {}} onFramesSelected={() => {}}
    onKeysSelected={() => {}} settings={null} capture={null} floor={floor} onCapture={() => {}} />);
  try {
    await act(async () => render(6, 4));
    const ticks = [...container.querySelectorAll<HTMLElement>(".tl-ruler .tl-grid .tl-tick")];
    expect(ticks.map((tick) => tick.classList.contains("tl-outside"))).toEqual(
      [true, true, true, true, false, false, false, false, false, false],
    );
    expect(move).not.toHaveBeenCalled();

    // A press at the ruler's left end asks for step 0 and gets the floor.
    const ruler = container.querySelector<HTMLElement>(".tl-ruler .tl-grid")!;
    await act(async () => ruler.dispatchEvent(new PointerEvent("pointerdown", {bubbles:true, cancelable:true, button:0, clientX:0})));
    expect(move).toHaveBeenLastCalledWith(4);
    await act(async () => window.dispatchEvent(new PointerEvent("pointerup")));

    // A playhead found behind a floor is brought up to it.
    move.mockClear();
    await act(async () => render(2, 4));
    expect(move).toHaveBeenLastCalledWith(4);

    // And with the floor gone nothing is dimmed and the ruler is free again.
    move.mockClear();
    await act(async () => render(6, null));
    expect(container.querySelectorAll(".tl-ruler .tl-grid .tl-tick.tl-outside").length).toBe(0);
    await act(async () => ruler.dispatchEvent(new PointerEvent("pointerdown", {bubbles:true, cancelable:true, button:0, clientX:0})));
    expect(move).toHaveBeenLastCalledWith(0);
    await act(async () => window.dispatchEvent(new PointerEvent("pointerup")));
  } finally { await act(async () => root.unmount()); container.remove(); }
});

it.each(["macro", "patch", "liquify"])("does not offer shape animation for a %s", async (tool) => {
  backend.tree.mockResolvedValue({ layers: [{id:1, name:"Layer", visible:true, locked:false, source:"painted", parameter:"wind", grib:null, image:null, gis:null, sst:null,
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

it("clears old viewport readiness immediately and ignores its late reports", async () => {
  vi.useFakeTimers();
  const project = { revision: 7, step_count: 10, step_hours: 1, start_unix_s: null } as ProjectSummary;
  const complete = { revision: 7, steps: Array.from({ length: 10 }, (_, step) => ({ step, ready: 1, total: 1 })) };
  backend.readiness.mockResolvedValue(complete);
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  const prepare = vi.fn((_request: import("./preparation").PreparationRequest) => ({ ready: 0, total: 10, streaming: false }));
  const render = (x: number) => root.render(<Timeline project={project} step={0} onStepChange={() => {}}
    selection={[]} onSelect={() => {}} viewport={[{ z: 1, x, y: 0 }]}
    playback={{ prepare, present: () => false }} autoKey={false} onAutoKey={() => {}}
    onChanged={() => {}} onFramesSelected={() => {}} onKeysSelected={() => {}} settings={null}
    capture={null} onCapture={() => {}} />);
  try {
    await act(async () => render(0));
    expect(container.querySelectorAll(".tl-tick.solid")).toHaveLength(10);
    let oldReport!: (value: typeof complete) => void;
    backend.readiness.mockImplementationOnce(() => new Promise(resolve => { oldReport = resolve; }));
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    let newReport!: (value: typeof complete) => void;
    backend.readiness.mockImplementationOnce(() => new Promise(resolve => { newReport = resolve; }));
    await act(async () => render(1));
    expect(container.querySelectorAll(".tl-tick.solid")).toHaveLength(0);
    expect(prepare.mock.calls.at(-1)?.[0].states).toEqual([]);
    await act(async () => oldReport(complete));
    expect(container.querySelectorAll(".tl-tick.solid")).toHaveLength(0);
    await act(async () => newReport(complete));
    await act(async () => { await vi.advanceTimersByTimeAsync(100); });
    expect(container.querySelectorAll(".tl-tick.solid")).toHaveLength(10);
    expect(prepare.mock.calls.at(-1)?.[0].states).toEqual(Array(10).fill("solid"));
    expect(backend.renderAhead).toHaveBeenLastCalledWith(0, [{ z: 1, x: 1, y: 0 }]);
  } finally {
    await act(async () => root.unmount()); container.remove();
  }
});

it("shows relative displacement as one position-style track with shared keys", async () => {
  backend.tree.mockResolvedValue({layers:[{id:1,name:"Layer",visible:true,locked:false,source:"painted",parameter:"wind",grib:null,image:null,gis:null,sst:null,
    objects:[{id:2,name:"Liquify 1",tool:"liquify",tool_label:"Liquify",active_here:true,start_step:0,end_step:9}]}]});
  backend.tracks.mockResolvedValue({object:2,tracks:[{
    property:"DisplacementPosition",label:"Displacement position",base:{kind:"offset",x:200,y:-100},keys:[],
    interpolations:[{kind:"linear"}],keyed_here:false,interpolated_here:false,motion_available:false,motion:false,can_follow:false,follows:null,follows_name:null,inherited:[],
  }]});
  const project={revision:21,step_count:10,step_hours:1,start_unix_s:null} as ProjectSummary;
  backend.setKey.mockResolvedValue(project);
  const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
  try {
    await act(async()=>root.render(<Timeline project={project} step={3} onStepChange={()=>{}} selection={[]} onSelect={()=>{}}
      viewport={[]} playback={{prepare:()=>({ready:0,total:0,streaming:false}),present:()=>false}} autoKey={false} onAutoKey={()=>{}} onChanged={()=>{}} onFramesSelected={()=>{}}
      onKeysSelected={()=>{}} settings={null} capture={null} onCapture={()=>{}} />));
    await act(async()=>container.querySelector<HTMLButtonElement>('[title="Show properties"]')!.click());
    expect([...container.querySelectorAll(".tl-track-name")].map(e=>e.textContent)).toEqual(["Displacement position"]);
    expect(container.querySelector(".tl-track .tl-graph-toggle")?.tagName).toBe("BUTTON");
    await act(async()=>container.querySelector<HTMLButtonElement>(".tl-key-here")!.click());
    expect(backend.setKey).toHaveBeenLastCalledWith(2,"DisplacementPosition",3);
  } finally {await act(async()=>root.unmount());container.remove();backend.tree.mockResolvedValue({layers:[]});}
});

/**
 * A file whose times disagree with the timeline's is marked and nothing
 * more (spec.md 4.8, M91): the row is highlighted, the tooltip says by how
 * much, and the small button beside the name asks the backend to align it —
 * unless no step lands on the file's time, when it is there but disabled.
 */
it("marks a misaligned layer and offers to align it", async () => {
  const steps = Array.from({ length: 10 }, (_, s) => ({ in_file: s < 3, source: null, hidden: false, shown: s < 3 }));
  const grib = (misaligned: import("../generated/LayerAlignment").LayerAlignment | null) => ({
    path: "x.grib2", history: null, field_kind: "wind", loaded: true, frame_count: 3, span_hours: 6,
    speed_min_mps: null, speed_max_mps: null, speed_ceiling_mps: 9, covered_steps: steps.map((s) => s.in_file), steps, misaligned,
  });
  const layer = (id: number, name: string, misaligned: import("../generated/LayerAlignment").LayerAlignment | null) => ({
    id, name, visible: true, locked: false, objects: [], image: null, gis: null, sst: null, source: "raster", parameter: "wind", grib: grib(misaligned),
  });
  backend.tree.mockResolvedValue({ layers: [
    layer(1, "Late", { first_valid_unix_s: 1_790_823_600, step_unix_s: 1_790_812_800, offset_hours: 3, aligned_lead: 1, lead_steps: 0, undated: false }),
    layer(2, "Odd", { first_valid_unix_s: 1_790_816_400, step_unix_s: 1_790_812_800, offset_hours: 1, aligned_lead: null, lead_steps: 0, undated: false }),
    layer(3, "Fine", null),
  ] } as unknown as { layers: import("../generated/LayerNode").LayerNode[] });
  const project = {revision: 22, step_count: 10, step_hours: 3, start_unix_s: 1_790_812_800} as ProjectSummary;
  const changed = vi.fn();
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Timeline project={project} step={0} onStepChange={() => {}} selection={[]} onSelect={() => {}}
      viewport={[]} playback={{prepare: () => ({ready:0,total:0,streaming:false}), present: () => false}} autoKey={false} onAutoKey={() => {}} onChanged={changed} onFramesSelected={() => {}}
      onKeysSelected={() => {}} settings={null} capture={null} onCapture={() => {}} />));
    // The timeline lists the top of the stack first.
    const rows = [...container.querySelectorAll<HTMLElement>(".tl-layer-row")].reverse();
    expect(rows.map((row) => row.classList.contains("tl-misaligned"))).toEqual([true, true, false]);
    expect(rows[0]!.title).toContain("step 0 (2026-10-01 00:00 UTC)");
    expect(rows[0]!.title).toContain("valid at 2026-10-01 03:00 UTC");

    expect(rows[0]!.title).toContain("3 h off");
    const buttons = rows.map((row) => row.querySelector<HTMLButtonElement>(".tl-align"));
    expect(buttons.map((b) => b?.disabled ?? "absent")).toEqual([false, true, "absent"]);
    expect(buttons[1]!.title).toContain("1 h is not a whole number of 3 h steps");
    await act(async () => buttons[0]!.click());
    expect(backend.align).toHaveBeenCalledWith(1);
    expect(changed).toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); container.remove(); }
});
