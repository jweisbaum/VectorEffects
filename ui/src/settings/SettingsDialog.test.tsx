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
import type { ProjectSummary } from "../generated/ProjectSummary";

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
  const gradients = [
    {
      id: "vector",
      label: "VectorEffects",
      note: "The application's own.",
      stops: [
        [0, 0, 0],
        [1, 1, 1],
      ],
    },
    {
      id: "viridis",
      label: "Viridis",
      note: "Perceptually uniform.",
      stops: [
        [0, 0, 0],
        [0.5, 0.5, 0.5],
        [1, 1, 1],
      ],
    },
  ];
  // Only the fields the dialog reads. A whole summary would be forty
  // properties of which four are under test, and the other thirty-six would
  // say nothing about whether the list renders.
  const project = {
    wind_scale_knots: 60,
    current_scale_knots: 6,
    wind_gradient: "vector",
    current_gradient: "viridis",
  };
  return {
    full,
    empty,
    deleted: [] as Array<string | null>,
    library: full,
    gradients,
    project,
    chosen: [] as Array<[string, string]>,
  };
});

vi.mock("../ipc", () => ({
  api: {
    // The dialog asks for the gradient catalogue on mount (M42). An empty
    // one is a real state — the first frames see it — and the dialog has to
    // render through it.
    colourGradients: () => Promise.resolve(held.gradients),
    setColourGradient: (kind: string, gradient: string) => {
      held.chosen.push([kind, gradient]);
      return Promise.resolve(held.project);
    },
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
  held.chosen.length = 0;
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

/** Renders with a project, so the per-project sections appear. */
async function renderWithProject(overrides: Record<string, unknown> = {}) {
  await act(async () => {
    root.render(
      <SettingsDialog
        settings={settings}
        project={{ ...held.project, ...overrides } as unknown as ProjectSummary}
        onSettings={() => undefined}
        onProject={() => undefined}
        onLibrary={() => undefined}
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

/**
 * Choosing a gradient (M42).
 *
 * The point of the picker is that it *shows* the gradients: a column of names
 * would ask the reader to remember what each one looks like, which is what
 * they opened the settings to find out. So what is asserted is that opening
 * it lists them with their own colours, that the closed control shows the
 * chosen one, and that the two kinds are chosen independently.
 */
describe("the colour gradient picker", () => {
  /** The trigger for one kind, by the label beside it. */
  function trigger(kind: "wind" | "current"): HTMLButtonElement {
    const word = kind === "wind" ? "for wind" : "for currents";
    const found = [...container.querySelectorAll(".gradient-picker")].find((picker) =>
      (picker.querySelector(".gradient-picker-label")?.textContent ?? "").includes(word),
    );
    const button = found?.querySelector("button");
    if (!button) throw new Error(`no gradient picker for ${kind}`);
    return button as HTMLButtonElement;
  }

  /** The open list's options, or none when it is closed. */
  const options = () => [...container.querySelectorAll<HTMLElement>('[role="option"]')];

  it("is closed until it is opened, and then lists every gradient", async () => {
    await renderWithProject();
    expect(options(), "closed to begin with").toHaveLength(0);
    expect(trigger("wind").getAttribute("aria-expanded")).toBe("false");

    await click(trigger("wind"));
    expect(options().map((row) => row.textContent)).toEqual(["VectorEffects", "Viridis"]);
    expect(trigger("wind").getAttribute("aria-expanded")).toBe("true");
  });

  it("shows each gradient's own colours, not one run for all of them", async () => {
    await renderWithProject();
    await click(trigger("wind"));
    const bars = options().map((row) => row.querySelector<HTMLElement>(".gradient-bar"));
    expect(bars[0]?.style.background).toContain("#000000");
    expect(bars[0]?.style.background).toContain("#ffffff");
    expect(bars[1]?.style.background).toContain("#808080");
    expect(bars[0]?.style.background).not.toBe(bars[1]?.style.background);
  });

  /** Closed, the control still says which gradient is in use, and shows it. */
  it("shows the chosen gradient when it is closed", async () => {
    await renderWithProject();
    expect(trigger("wind").textContent).toContain("VectorEffects");
    expect(trigger("current").textContent).toContain("Viridis");
    expect(
      trigger("current").querySelector<HTMLElement>(".gradient-bar")?.style.background,
    ).toContain("#808080");
  });

  it("marks the one in use as selected", async () => {
    await renderWithProject();
    await click(trigger("current"));
    const selected = options().filter((row) => row.getAttribute("aria-selected") === "true");
    expect(selected.map((row) => row.textContent)).toEqual(["Viridis"]);
  });

  it("writes the one that is chosen, for that kind alone, and closes", async () => {
    await renderWithProject();
    await click(trigger("wind"));
    const viridis = options().find((row) => row.textContent === "Viridis");
    expect(viridis).toBeDefined();
    await click(viridis as HTMLElement);
    expect(held.chosen).toEqual([["wind", "viridis"]]);
    expect(options(), "and the list closes behind it").toHaveLength(0);
  });

  /**
   * A project written by a later version names a gradient this build does not
   * have. It is listed, because that is what the file says, and it cannot be
   * picked, because there is nothing to pick.
   */
  it("shows a gradient it does not have, and refuses to pick it", async () => {
    await renderWithProject({ wind_gradient: "from-a-later-version" });
    expect(trigger("wind").textContent).toContain("from-a-later-version");
    await click(trigger("wind"));
    const unknown = options().find((row) =>
      (row.textContent ?? "").startsWith("from-a-later-version"),
    );
    expect(unknown?.getAttribute("aria-disabled")).toBe("true");
    expect(unknown?.getAttribute("aria-selected")).toBe("true");
    await click(unknown as HTMLElement);
    expect(held.chosen, "clicking it writes nothing").toEqual([]);
  });

  it("closes on Escape without choosing anything", async () => {
    await renderWithProject();
    await click(trigger("wind"));
    expect(options().length).toBeGreaterThan(0);
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(options()).toHaveLength(0);
    expect(held.chosen).toEqual([]);
  });
});

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
