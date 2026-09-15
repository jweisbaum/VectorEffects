// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { ProjectSummary } from "./generated/ProjectSummary";
import type { DocumentTree } from "./generated/DocumentTree";
import { useSelectionLifecycle } from "./selectionLifecycle";
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const backend = vi.hoisted(() => ({ tree: vi.fn() }));
vi.mock("./ipc", () => ({ api: { documentTree: backend.tree } }));
it("retires deleted selection without a layer panel and ignores an older document reply", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const select = vi.fn();
  let old!: (tree: DocumentTree) => void;
  backend.tree.mockReturnValueOnce(new Promise(resolve => { old = resolve; }));
  function Host({ revision, ids }: { revision: number; ids: number[] }) {
    useSelectionLifecycle({ revision, image_token: 1 } as ProjectSummary, ids, select);
    return null;
  }
  try {
    await act(async () => root.render(<Host revision={1} ids={[50]} />));
    backend.tree.mockResolvedValueOnce({ layers: [{ objects: [{ id: 51 }] }] });
    await act(async () => root.render(<Host revision={2} ids={[51]} />));
    await act(async () => old({ layers: [] }));
    expect(select).not.toHaveBeenCalled();
    backend.tree.mockResolvedValueOnce({ layers: [] });
    await act(async () => root.render(<Host revision={3} ids={[51]} />));
    expect(select).toHaveBeenCalledTimes(1);
    expect(select).toHaveBeenCalledWith([]);
  } finally { await act(async () => root.unmount()); }
});
