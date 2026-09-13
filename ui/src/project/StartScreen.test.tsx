// @vitest-environment happy-dom
/**
 * Clearing the recent list is asked for, and says so.
 *
 * The list is the start screen's only navigation aid and clearing it cannot be
 * undone, so the button opens a confirmation rather than doing it — and
 * nothing is cleared until that confirmation is answered.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { RecentProject } from "../generated/RecentProject";

// React only records updates as acted upon when told it is in a test.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const held = vi.hoisted(() => {
  const entry = (name: string): RecentProject => ({
    path: `/projects/${name}.veproj`,
    name,
    exists: true,
  });
  return {
    recent: [entry("cyclone"), entry("gyre")],
    /** How many times the backend was actually asked to clear. */
    clears: 0,
    /** When set, the clear fails with this message instead of succeeding. */
    refuseWith: null as string | null,
  };
});

vi.mock("../ipc", () => ({
  IpcError: class IpcError extends Error {},
  api: {
    recentProjects: () => Promise.resolve(held.recent),
    autosaves: () => Promise.resolve([]),
    clearRecentProjects: () => {
      held.clears += 1;
      if (held.refuseWith !== null) return Promise.reject(new Error(held.refuseWith));
      held.recent = [];
      return Promise.resolve(held.recent);
    },
  },
}));

// The native file dialogs reach for the Tauri plugin, which is not there in a
// test. The start screen never opens one in these cases.
vi.mock("./dialogs", () => ({
  pickProjectToOpen: () => Promise.resolve(null),
  pickGribToImport: () => Promise.resolve(null),
}));

const StartScreen = (await import("./StartScreen")).default;

let container: HTMLDivElement;
let root: Root;

/** Every button on the screen whose label is exactly `label`. */
function buttons(label: string): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll("button")).filter(
    (button) => button.textContent?.trim() === label,
  );
}

/** The one button labelled `label`, or a failure naming what was found. */
function button(label: string): HTMLButtonElement {
  const found = buttons(label);
  if (found.length !== 1) throw new Error(`expected one “${label}”, found ${found.length}`);
  return found[0] as HTMLButtonElement;
}

async function click(element: HTMLButtonElement) {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

async function render() {
  await act(async () => {
    root.render(<StartScreen onOpened={() => undefined} />);
  });
}

beforeEach(async () => {
  held.recent = [
    { path: "/projects/cyclone.veproj", name: "cyclone", exists: true },
    { path: "/projects/gyre.veproj", name: "gyre", exists: true },
  ];
  held.clears = 0;
  held.refuseWith = null;
  container = document.createElement("div");
  document.body.appendChild(container);
  await act(async () => {
    root = createRoot(container);
  });
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("clearing the recent projects list", () => {
  it("offers the button only when there is something to clear", async () => {
    await render();
    expect(buttons("Clear")).toHaveLength(1);

    held.recent = [];
    await act(async () => root.unmount());
    await act(async () => {
      root = createRoot(container);
    });
    await render();

    expect(buttons("Clear")).toHaveLength(0);
  });

  it("asks before clearing, and clears nothing while it asks", async () => {
    await render();
    await click(button("Clear"));

    expect(container.textContent).toContain("Clear recent projects");
    expect(held.clears).toBe(0);
    expect(container.textContent).toContain("cyclone");
  });

  it("clears nothing when the confirmation is cancelled", async () => {
    await render();
    await click(button("Clear"));
    await click(button("Cancel"));

    expect(held.clears).toBe(0);
    expect(container.textContent).toContain("cyclone");
    expect(container.textContent).not.toContain("Clear recent projects");
  });

  it("empties the list once the confirmation is accepted", async () => {
    await render();
    await click(button("Clear"));
    // Two now carry the label: the one in the list header and the dialog's.
    const confirm = buttons("Clear")[1];
    if (confirm === undefined) throw new Error("no confirming button");
    await click(confirm);

    expect(held.clears).toBe(1);
    expect(container.textContent).not.toContain("cyclone");
    expect(buttons("Clear")).toHaveLength(0);
  });

  /**
   * The write can fail — the settings file is on disk. The list must not be
   * emptied on screen when the backend did not empty it, or the next launch
   * appears to bring it back.
   */
  it("keeps the list and reports the error when the write is refused", async () => {
    held.refuseWith = "read-only file system";
    await render();
    await click(button("Clear"));
    const confirm = buttons("Clear")[1];
    if (confirm === undefined) throw new Error("no confirming button");
    await click(confirm);

    expect(held.clears).toBe(1);
    expect(container.textContent).toContain("cyclone");
    expect(container.textContent).toContain("read-only file system");
  });
});
