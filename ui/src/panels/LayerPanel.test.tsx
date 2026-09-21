// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { DocumentTree } from "../generated/DocumentTree";
import type { ProjectSummary } from "../generated/ProjectSummary";
import LayerPanel from "./LayerPanel";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const backend = vi.hoisted(() => ({
  documentTree: vi.fn(), moveLayer: vi.fn(), moveObject: vi.fn(), setLayerVisible: vi.fn(),
  importZarr: vi.fn(), pickZarr: vi.fn(),
}));
vi.mock("../ipc", () => ({ api: backend }));
vi.mock("../hint", () => ({ reportError: vi.fn() }));
vi.mock("../project/dialogs", () => ({
  pickZarrToImport: backend.pickZarr,
  pickGribToImport: vi.fn(),
  pickImageToImport: vi.fn(),
}));

let container: HTMLDivElement;
let root: Root;
let tree: DocumentTree;
let hit: Element | null;
const activate = vi.fn();
const project = { revision: 1, step_count: 8, step_hours: 1 } as ProjectSummary;

function Host() {
  const [summary, setSummary] = useState(project);
  return <LayerPanel project={summary} step={0} selection={[]} activeLayer={4}
    onSelect={() => {}} onActivateLayer={activate} onActiveKind={() => {}} onChanged={setSummary}
    onAlign={() => {}}
      canAlign />;
}

function row(id: number): HTMLElement {
  return container.querySelector<HTMLElement>(`[data-layer-id="${id}"] .layer-header`)!;
}

function pointer(target: EventTarget, type: string, y: number, pointerId = 1) {
  target.dispatchEvent(new PointerEvent(type, {
    bubbles: true, cancelable: true, pointerId, isPrimary: true,
    pointerType: "mouse", button: 0, buttons: type === "pointerup" ? 0 : 1, clientX: 50, clientY: y,
  }));
}

beforeEach(async () => {
  vi.clearAllMocks();
  backend.pickZarr.mockResolvedValue(null);
  backend.importZarr.mockResolvedValue({ ...project, revision: 2 });
  tree = { layers: ["painted", "raster", "zarr", "image"].map((source, i) => ({
    id: i + 1, name: `Layer ${i + 1}`, source, visible: true, locked: false,
    objects: [], grib: null, image: null, gis: null, parameter: "wind",
  })) };
  backend.documentTree.mockImplementation(async () => structuredClone(tree));
  backend.moveLayer.mockImplementation(async (from: number, to: number) => {
    const [layer] = tree.layers.splice(from, 1);
    tree.layers.splice(to, 0, layer!);
    return { ...project, revision: 2 };
  });
  backend.moveObject.mockImplementation(async (id: number, layerId: number, to: number) => {
    const source = tree.layers.find((layer) => layer.objects.some((object) => object.id === id))!;
    const [object] = source.objects.splice(source.objects.findIndex((object) => object.id === id), 1);
    const destination = tree.layers.find((layer) => layer.id === layerId)!;
    destination.objects.splice(to, 0, object!);
    return { ...project, revision: 2 };
  });
  backend.setLayerVisible.mockResolvedValue({ ...project, revision: 2 });
  hit = null;
  vi.spyOn(document, "elementFromPoint").mockImplementation(() => hit);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 100, 200, 40));
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<Host />));
  activate.mockClear();
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

it("imports a selected Zarr directory through the layer controls", async () => {
  const button = container.querySelector<HTMLButtonElement>('button[title^="Import wind and currents"]')!;
  await act(async () => button.click());
  expect(backend.importZarr).not.toHaveBeenCalled();
  backend.pickZarr.mockResolvedValue("/data/routing_test");
  await act(async () => button.click());
  expect(backend.importZarr).toHaveBeenCalledWith("/data/routing_test");
});

it.each([
  [1, 4, 105, ".grip", [1, 4, 3, 2]],
  [4, 1, 135, ".name", [3, 2, 1, 4]],
  [2, 4, 105, ".name", [2, 4, 3, 1]],
  [3, 1, 135, ".grip", [4, 2, 1, 3]],
] as const)("drags layer %i beside layer %i and displays the new order", async (from, target, y, handle, order) => {
  await act(async () => pointer(row(from).querySelector(handle)!, "pointerdown", 50));
  hit = row(target).querySelector(".name");
  await act(async () => pointer(window, "pointermove", y));
  expect(row(target).classList.contains(y < 120 ? "drop-above" : "drop-below")).toBe(true);
  await act(async () => {
    pointer(window, "pointerup", y);
    row(from).click(); // The browser's click following a drag must not activate it.
  });
  expect(backend.moveLayer).toHaveBeenCalledTimes(1);
  expect([...container.querySelectorAll<HTMLElement>("[data-layer-id]")].map((el) => Number(el.dataset.layerId))).toEqual(order);
  expect(activate).not.toHaveBeenCalled();
});

it("prevents native dragging from taking over the pointer gesture", () => {
  const event = new Event("dragstart", { bubbles: true, cancelable: true });
  row(1).querySelector(".name")!.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(true);
});

it.each(["pointercancel", "Escape", "blur"])("cancels a drag on %s without moving a layer", async (cancel) => {
  await act(async () => pointer(row(1), "pointerdown", 50));
  hit = row(4);
  await act(async () => pointer(window, "pointermove", 105));
  await act(async () => {
    if (cancel === "Escape") window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    else if (cancel === "blur") window.dispatchEvent(new Event("blur"));
    else pointer(window, cancel, 105);
    pointer(window, "pointerup", 105);
  });
  expect(backend.moveLayer).not.toHaveBeenCalled();
  expect(container.querySelector(".drop-above, .drop-below")).toBeNull();
});

it("keeps buttons and rename inputs out of the drag gesture", async () => {
  await act(async () => row(1).querySelector(".name")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
  for (const control of row(1).querySelectorAll("button, input")) {
    hit = row(4);
    await act(async () => {
      pointer(control, "pointerdown", 50);
      pointer(window, "pointermove", 105);
      pointer(window, "pointerup", 105);
    });
  }
  expect(backend.moveLayer).not.toHaveBeenCalled();
  await act(async () => row(1).querySelector<HTMLButtonElement>(".eye")!.click());
  expect(backend.setLayerVisible).toHaveBeenCalledWith(1, false);
});

it("keeps ordinary row clicks working and ignores other pointers during a press", async () => {
  hit = row(4);
  await act(async () => {
    pointer(row(1), "pointerdown", 50);
    pointer(window, "pointermove", 105, 2);
    pointer(window, "pointerup", 105, 2);
    pointer(window, "pointerup", 50);
    row(1).click();
  });
  expect(backend.moveLayer).not.toHaveBeenCalled();
  expect(activate).toHaveBeenCalledWith(1);
});

it("moves an object even when move and release arrive before React renders", async () => {
  tree.layers[0]!.objects = [11, 12].map((id) => ({
    id, name: `Object ${id}`, tool: "brush", tool_label: "Brush",
    active_here: true, start_step: 0, end_step: 7,
  }));
  await act(async () => root.render(<Host key="objects" />));
  hit = container.querySelector('[data-object-id="12"]');
  await act(async () => {
    pointer(container.querySelector('[data-object-id="11"]')!, "pointerdown", 50);
    pointer(window, "pointermove", 105);
    pointer(window, "pointerup", 105);
  });
  expect(backend.moveObject).toHaveBeenCalledWith(11, 1, 1);
});

function objectRow(id: number): HTMLElement {
  return container.querySelector<HTMLElement>(`[data-object-id="${id}"]`)!;
}

function shownObjects(layerId: number): number[] {
  return [...container.querySelectorAll<HTMLElement>(`[data-layer-id="${layerId}"] [data-object-id]`)]
    .map((element) => Number(element.dataset.objectId));
}

async function withObjects() {
  for (const layer of tree.layers.slice(0, 2)) {
    layer.source = "painted";
    layer.objects = [1, 2, 3].map((n) => ({
      id: layer.id * 10 + n, name: `Object ${layer.id * 10 + n}`, tool: "brush", tool_label: "Brush",
      active_here: true, start_step: 0, end_step: 7,
    }));
  }
  await act(async () => root.render(<Host key="object-order" />));
}

it.each([
  [11, 13, 105, [11, 13, 12]],
  [11, 13, 135, [13, 11, 12]],
  [13, 11, 105, [12, 13, 11]],
  [13, 11, 135, [12, 11, 13]],
] as const)("places object %i on the indicated side of object %i", async (from, target, y, expected) => {
  await withObjects();
  await act(async () => pointer(objectRow(from).querySelector(".grip")!, "pointerdown", 50));
  hit = objectRow(target).querySelector(".name");
  await act(async () => pointer(window, "pointermove", y));
  expect(objectRow(target).classList.contains(y < 120 ? "drop-above" : "drop-below")).toBe(true);
  await act(async () => pointer(window, "pointerup", y));
  expect(shownObjects(1)).toEqual(expected);
  expect(shownObjects(2)).toEqual([23, 22, 21]);
  expect(backend.moveObject).toHaveBeenCalledTimes(1);
  expect(backend.moveLayer).not.toHaveBeenCalled();
});

it.each([[12, 12, 105], [12, 12, 135], [11, 12, 135], [13, 12, 105]])(
  "does not write an unchanged object order (%i beside %i)", async (from, target, y) => {
    await withObjects();
    hit = objectRow(target);
    await act(async () => {
      pointer(objectRow(from).querySelector(".name")!, "pointerdown", 50);
      pointer(window, "pointermove", y);
      pointer(window, "pointerup", y);
    });
    expect(backend.moveObject).not.toHaveBeenCalled();
    expect(shownObjects(1)).toEqual([13, 12, 11]);
  },
);

it.each(["pointercancel", "Escape"])("cancels an object reorder with %s", async (cancel) => {
  await withObjects();
  hit = objectRow(13);
  await act(async () => {
    pointer(objectRow(11).querySelector(".name")!, "pointerdown", 50);
    pointer(window, "pointermove", 105);
  });
  await act(async () => {
    if (cancel === "Escape") window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    else pointer(window, cancel, 105);
    pointer(window, "pointerup", 105);
  });
  expect(backend.moveObject).not.toHaveBeenCalled();
  expect(shownObjects(1)).toEqual([13, 12, 11]);
  expect(container.querySelector(".drop-above, .drop-below")).toBeNull();
});

it.each([[105, [23, 11, 22, 21]], [135, [23, 22, 11, 21]]] as const)(
  "inserts at the indicated edge in another layer (%i)", async (y, expected) => {
    await withObjects();
    hit = objectRow(22);
    await act(async () => {
      pointer(objectRow(11).querySelector(".name")!, "pointerdown", 50);
      pointer(window, "pointermove", y);
      pointer(window, "pointerup", y);
    });
    expect(shownObjects(1)).toEqual([13, 12]);
    expect(shownObjects(2)).toEqual(expected);
  },
);

it("drops an object onto its layer header to bring it to the top", async () => {
  await withObjects();
  hit = row(1);
  await act(async () => {
    pointer(objectRow(11).querySelector(".name")!, "pointerdown", 50);
    pointer(window, "pointermove", 105);
    pointer(window, "pointerup", 105);
  });
  expect(shownObjects(1)).toEqual([11, 13, 12]);
  expect(backend.moveObject).toHaveBeenCalledWith(11, 1, 2);
});
