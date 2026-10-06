// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectSummary } from "../generated/ProjectSummary";
import ExportZarrDialog from "./ExportZarrDialog";

vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("../ipc", () => ({ api: {}, IpcError: class extends Error {} }));

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

const project = (overrides: Partial<ProjectSummary>) =>
  ({
    name: "P",
    resolution_deg: 0.25,
    grid_ni: 1440,
    grid_nj: 721,
    step_hours: 3,
    step_count: 8,
    start_unix_s: null,
    region: null,
    ...overrides,
  }) as ProjectSummary;

describe("the Zarr export dialog", () => {
  it("states the store's global layout, not the project's lattice", async () => {
    await act(async () => root.render(<ExportZarrDialog project={project({})} onClose={() => {}} />));
    // The routing layout stops a row short of the south pole.
    expect(container.textContent).toContain("1440 × 720");
    expect(container.textContent).not.toContain("region's chunks");
  });

  it("says a regional project writes only its region's chunks of it", async () => {
    const regional = project({
      grid_ni: 81,
      grid_nj: 41,
      region: { west: -40, east: -20, south: 40, north: 50, full_circle: false },
    });
    await act(async () => root.render(<ExportZarrDialog project={regional} onClose={() => {}} />));
    expect(container.textContent).toContain("1440 × 720");
    expect(container.textContent).not.toContain("81 × 41");
    expect(container.textContent).toContain("Only the region's chunks are written");
  });
});
