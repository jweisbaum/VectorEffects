// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { NewProjectRequest } from "../generated/NewProjectRequest";
import type { RegionRequest } from "../generated/RegionRequest";
import NewProjectForm from "./NewProjectForm";
import RegionPicker from "./RegionPicker";
import { regionSpan } from "./regionPick";

// The coast is drawn from the bundled basemap; there is no backend here.
vi.mock("../ipc", () => ({ api: { basemap: () => Promise.reject(new Error("no backend")) } }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function input(label: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`);
  if (!found) throw new Error(`no field labelled ${label}`);
  return found;
}

/** What a person does: types the text, then leaves the field. */
async function type(field: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  await act(async () => {
    field.focus();
    setter?.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    field.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
  });
}

async function click(element: Element) {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function button(text: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll("button")).find((b) => b.textContent === text);
  if (!found) throw new Error(`no button ${text}`);
  return found;
}

const BOX: RegionRequest = { west: 10, east: 20, south: 40, north: 50, full_circle: false };

describe("the region picker's fields", () => {
  it("sends each typed edge snapped outward to the lattice", async () => {
    const seen: RegionRequest[] = [];
    await act(async () =>
      root.render(<RegionPicker value={BOX} resolution="0.25" onChange={(r) => seen.push(r)} />),
    );
    await type(input("North"), "49.9");
    expect(seen.at(-1)).toEqual({ ...BOX, north: 50 });
    await type(input("South"), "40.1");
    expect(seen.at(-1)).toEqual({ ...BOX, south: 40 });
    await type(input("West"), "10.1");
    expect(seen.at(-1)).toEqual({ ...BOX, west: 10 });
    await type(input("East"), "-170.1");
    expect(seen.at(-1)).toEqual({ ...BOX, east: -170 });
  });

  it("covers every longitude when Full circle is checked", async () => {
    const seen: RegionRequest[] = [];
    await act(async () =>
      root.render(<RegionPicker value={BOX} resolution="0.25" onChange={(r) => seen.push(r)} />),
    );
    await click(input("Full circle"));
    const last = seen.at(-1)!;
    expect(last.full_circle).toBe(true);
    expect(regionSpan(last)).toBe(360);
  });
});

describe("the new-project form", () => {
  it("refuses a full circle from pole to pole, which is a global project", async () => {
    const submitted: NewProjectRequest[] = [];
    await act(async () =>
      root.render(<NewProjectForm submitLabel="Create project" onSubmit={(r) => submitted.push(r)} />),
    );
    const extent = container.querySelector<HTMLSelectElement>('[data-feature="new:extent"] select')!;
    await act(async () => {
      extent.value = "regional";
      extent.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await click(button("Arctic"));
    expect(container.textContent).not.toContain("Choose Global for the whole earth");
    expect(button("Create project").disabled).toBe(false);

    await type(input("South"), "-90");
    expect(container.textContent).toContain("Choose Global for the whole earth");
    expect(button("Create project").disabled).toBe(true);
  });

  it("sends the region with the request, and none for Global", async () => {
    const submitted: NewProjectRequest[] = [];
    await act(async () =>
      root.render(<NewProjectForm submitLabel="Create project" onSubmit={(r) => submitted.push(r)} />),
    );
    await click(button("Create project"));
    expect(submitted.at(-1)!.region).toBeNull();

    const extent = container.querySelector<HTMLSelectElement>('[data-feature="new:extent"] select')!;
    await act(async () => {
      extent.value = "regional";
      extent.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await click(button("Antarctic"));
    await click(button("Create project"));
    expect(submitted.at(-1)!.region).toEqual({ west: -180, east: 180, south: -90, north: -60, full_circle: true });
  });

  it("takes the open map's view when asked", async () => {
    const submitted: NewProjectRequest[] = [];
    await act(async () =>
      root.render(
        <NewProjectForm
          submitLabel="Create project"
          onSubmit={(r) => submitted.push(r)}
          viewBounds={() => [150, 20, 210, -20]}
        />,
      ),
    );
    const extent = container.querySelector<HTMLSelectElement>('[data-feature="new:extent"] select')!;
    await act(async () => {
      extent.value = "regional";
      extent.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await click(button("Use current view"));
    await click(button("Create project"));
    expect(submitted.at(-1)!.region).toEqual({ west: 150, east: -150, south: -20, north: 20, full_circle: false });
  });
});
