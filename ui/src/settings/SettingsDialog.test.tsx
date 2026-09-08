// @vitest-environment happy-dom
/**
 * Deleting the macro library is asked for, and says so (M35).
 *
 * It reaches outside the project, deletes files and cannot be undone, so the
 * button opens a confirmation rather than doing it — and nothing is deleted
 * until the confirmation is answered.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppSettings } from "../generated/AppSettings";
import type { MacroLibrary } from "../generated/MacroLibrary";

// React only records updates as acted upon when told it is in a test.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const held = vi.hoisted(() => {
  const entry = (id: string) => ({
    id,
    name: id,
    field_kind: "wind",
    frames: 3,
    span_hours: 6,
    step_hours: 3,
    size_bytes: 1024,
    moves: false,
    outline: { kind: "rect" as const, half_width_deg: 4, half_height_deg: 4 },
    track: [] as Array<[number, number]>,
  });
  const full: MacroLibrary = {
    directory: "/macros",
    entries: [entry("one"), entry("two")],
    total_bytes: 2048,
  };
  const empty: MacroLibrary = { directory: "/macros", entries: [], total_bytes: 0 };
  return { full, empty, deleted: [] as Array<string | null>, library: full };
});

vi.mock("../ipc", () => ({
  api: {
    // The dialog asks for the gradient catalogue on mount (M42). An empty
    // one is a real state — the first frames see it — and the dialog has to
    // render through it.
    colourGradients: () => Promise.resolve([]),
    macroLibrary: () => Promise.resolve(held.library),
    deleteMacros: (id: string | null) => {
      held.deleted.push(id);
      held.library = held.empty;
      return Promise.resolve(held.empty);
    },
  },
}));

const SettingsDialog = (await import("./SettingsDialog")).default;

const settings: AppSettings = {
  shortcuts: [],
  autosave: "recovery",
  default_wind_scale_knots: 60,
  default_current_scale_knots: 6,
  macro_directory: "/macros",
  projection: "equirectangular",
  auto_scale: false,
};

let container: HTMLDivElement;
let root: Root;
let libraryChanges: number;

beforeEach(() => {
  held.library = held.full;
  held.deleted.length = 0;
  libraryChanges = 0;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => {
    root.render(
      <SettingsDialog
        settings={settings}
        project={null}
        onSettings={() => undefined}
        onProject={() => undefined}
        onLibrary={() => {
          libraryChanges += 1;
        }}
        onClose={() => undefined}
      />,
    );
  });
}

/** The first button whose label starts with `text`. */
function button(text: string): HTMLButtonElement {
  const found = [...container.querySelectorAll("button")].find((el) =>
    (el.textContent ?? "").startsWith(text),
  );
  if (!found) throw new Error(`no button labelled ${text}`);
  return found as HTMLButtonElement;
}

async function click(el: HTMLElement) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

describe("deleting the macro library", () => {
  it("asks before deleting, and deletes nothing until it is answered", async () => {
    await render();
    await click(button("Delete all macros"));
    expect(held.deleted, "nothing is deleted by opening the confirmation").toEqual([]);

    // The confirmation names what goes.
    const confirm = container.querySelector('[aria-label="Delete all macros"]');
    expect(confirm?.textContent).toContain("Delete 2 macros?");
    expect(confirm?.textContent, "and how much disk it frees").toContain("2 kB");
  });

  it("deletes on the confirmation, and says the library has changed", async () => {
    await render();
    await click(button("Delete all macros"));
    await click(button("Delete 2 macros"));
    expect(held.deleted, "the whole library, not one entry").toEqual([null]);
    expect(libraryChanges, "so the insert tool re-reads it").toBe(1);
    expect(container.querySelector('[aria-label="Delete all macros"]')).toBeNull();
  });

  it("deletes nothing when the confirmation is cancelled", async () => {
    await render();
    await click(button("Delete all macros"));
    await click(button("Cancel"));
    expect(held.deleted).toEqual([]);
    expect(libraryChanges).toBe(0);
    expect(container.querySelector('[aria-label="Delete all macros"]')).toBeNull();
  });
});
