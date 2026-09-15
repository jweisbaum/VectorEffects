// @vitest-environment happy-dom
/**
 * What the properties panel says when no object is selected (M55).
 *
 * It said nothing at all — while the facts about a layer that are editable
 * nowhere, a history layer's archive and hours above all, had no home in the
 * interface. Those have been carried on the layer since M38 with nothing to
 * display them.
 *
 * Facts and not controls: the layer's name, eye, lock, filter and opacity are
 * edited on its own row in the layer panel, and a second copy here would be
 * two places to change one thing. So what is asserted is that the panel
 * *describes* the layer, and that it says the things nothing else does.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LayerNode } from "../generated/LayerNode";
import type { ProjectSummary } from "../generated/ProjectSummary";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const held = vi.hoisted(() => ({ layers: [] as unknown[], properties: vi.fn(async (_id: number) => [] as import("../generated/PropertyView").PropertyView[]), error: vi.fn() }));

vi.mock("../ipc", () => ({
  api: {
    documentTree: () => Promise.resolve({ layers: held.layers }),
    objectProperties: held.properties,
  },
}));

vi.mock("../hint", () => ({ reportError: held.error }));

const Inspector = (await import("./Inspector")).default;

/** A layer with only the fields the panel reads. */
function layer(over: Record<string, unknown>): LayerNode {
  return {
    id: 1,
    name: "Layer 1",
    visible: true,
    locked: false,
    source: "painted",
    parameter: "wind",
    grib: null,
    image: null,
    objects: [],
    ...over,
  } as unknown as LayerNode;
}

const project = { revision: 7 } as unknown as ProjectSummary;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function show(node: LayerNode, selection: number[] = []) {
  held.layers = [node];
  await act(async () => {
    root.render(
      <Inspector
        project={project}
        selection={selection}
        step={0}
        activeLayer={node.id}
        autoKey={false}
        picking={null}
        onPick={() => undefined}
        onChanged={() => undefined}
      />,
    );
  });
}

const text = () => container.textContent ?? "";

describe("the panel with no object selected", () => {
  it("names the layer and what it holds", async () => {
    await show(layer({ name: "Trade winds" }));
    expect(text()).toContain("Trade winds");
    expect(text()).toContain("Painted");
  });

  /**
   * The reason this panel exists: a history layer has carried its archive and
   * its hours since M38 and nothing has ever shown them.
   */
  it("says which archive a history layer came from, and which hours", async () => {
    await show(
      layer({
        name: "ERA5 10 m wind",
        source: "zarr",
        grib: {
          path: "/history/era5-wind.grib2",
          history: {
            archive: "era5-wind",
            label: "ERA5 10 m wind",
            start_unix_s: 1_757_203_200,
            end_unix_s: 1_757_289_600,
          },
          field_kind: "wind",
          loaded: true,
          frame_count: 8,
          span_hours: 21,
          speed_min_mps: null,
          speed_max_mps: null,
          speed_ceiling_mps: 30,
          covered_steps: [true, true, false],
          steps: [],
        },
      }),
    );
    expect(text()).toContain("History");
    expect(text()).toContain("ERA5 10 m wind");
    expect(text(), "the range, as dates rather than seconds").toContain("2025-09-07");
    expect(text(), "and how much of the timeline it reaches").toContain("2 of 3");
  });

  /** An imported forecast has no archive, and must not claim one. */
  it("says nothing about an archive for an imported forecast", async () => {
    await show(
      layer({
        source: "raster",
        grib: {
          path: "/tmp/gfs.grib2",
          history: null,
          field_kind: "wind",
          loaded: true,
          frame_count: 4,
          span_hours: 9,
          speed_min_mps: null,
          speed_max_mps: null,
          speed_ceiling_mps: 30,
          covered_steps: [true],
          steps: [],
        },
      }),
    );
    expect(text()).toContain("Imported field");
    expect(text()).toContain("gfs.grib2");
    expect(text()).not.toContain("Archive");
  });

  /** A file that would not read says so, rather than reading as empty. */
  it("says when a file could not be read", async () => {
    await show(
      layer({
        source: "raster",
        grib: {
          path: "/gone.grib2",
          history: null,
          field_kind: "wind",
          loaded: false,
          frame_count: 0,
          span_hours: 0,
          speed_min_mps: null,
          speed_max_mps: null,
          speed_ceiling_mps: 0,
          covered_steps: [],
          steps: [],
        },
      }),
    );
    expect(text()).toContain("missing or unreadable");
  });

  /** A selected object still gets the object's properties, not the layer's. */
  it("steps aside when an object is selected", async () => {
    await show(layer({ name: "Trade winds" }), [42]);
    expect(text()).not.toContain("Trade winds");
  });
});

it("ignores late properties and missing-object errors after changing selection", async () => {
  let resolveOld!: (rows: import("../generated/PropertyView").PropertyView[]) => void;
  held.properties.mockReturnValueOnce(new Promise(resolve => { resolveOld = resolve; }));
  await show(layer({}), [50]);
  held.properties.mockResolvedValueOnce([]);
  await show(layer({}), [51]);
  await act(async () => resolveOld([{ id: "Speed", label: "Stale speed", value: { kind: "number", value: 10 },
    unit: "speed", min: 0, max: 120, variants: [], animated: false, keyed_here: false,
    interpolated_here: false, keyable: true, slider: null }]));
  expect(text()).not.toContain("Stale speed");
  held.error.mockClear();
  held.properties.mockRejectedValueOnce({ kind: "missing-object", message: "gone" });
  await show(layer({}), [52]);
  expect(held.error).not.toHaveBeenCalled();
  held.properties.mockRejectedValueOnce(new Error("real read failure"));
  await show(layer({}), [53]);
  expect(held.error).toHaveBeenCalledWith("Error: real read failure");
});
