// @vitest-environment happy-dom
/**
 * The near-real-time dialog (spec 4.10, M89): what it offers and what it asks
 * the backend for.
 *
 * The request is the contract. The dialog sends a number of days and three
 * choices, and the import decides the hours from its own clock — so what is
 * held here is that the choices on screen are the ones sent, and that the
 * lengthen box exists exactly when the period needs more steps than the
 * project has. The counts are the hand-worked ones of `nrtRange.test.ts`.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { NrtRequest } from "../generated/NrtRequest";
import type { ProjectSummary } from "../generated/ProjectSummary";
import NrtImportDialog from "./NrtImportDialog";

// React only records updates as acted upon when told it is in a test.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** 2026-10-02T15:40Z: three days back is 88 hourly steps. */
const NOW = 1_790_955_600;

/** As much of a project as the dialog reads. */
function projectOf(timeline: {
  step_hours: number;
  step_count: number;
  start_unix_s: number | null;
}): ProjectSummary {
  return timeline as ProjectSummary;
}

let container: HTMLDivElement;
let root: Root;
let sent: NrtRequest[];
let closed: number;

async function open(project: ProjectSummary) {
  await act(async () =>
    root.render(
      <NrtImportDialog
        project={project}
        now={NOW}
        onImport={(request) => sent.push(request)}
        onClose={() => {
          closed += 1;
        }}
      />,
    ),
  );
}

/** The dialog is a portal on the body, not a child of the container. */
function dialog(): HTMLElement {
  const found = document.body.querySelector<HTMLElement>('[data-feature="nrt-import:dialog"]');
  if (found === null) throw new Error("the dialog is not on screen");
  return found;
}

function feature(name: string): HTMLElement | null {
  return dialog().querySelector<HTMLElement>(`[data-feature="nrt-import:${name}"]`);
}

function checkbox(name: string): HTMLInputElement {
  const found = feature(name)?.querySelector<HTMLInputElement>('input[type="checkbox"]');
  if (!found) throw new Error(`no checkbox in ${name}`);
  return found;
}

function products(): HTMLInputElement[] {
  return Array.from(
    feature("products")?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]') ?? [],
  );
}

function importButton(): HTMLButtonElement {
  const found = feature("import");
  if (!(found instanceof HTMLButtonElement)) throw new Error("no Import button");
  return found;
}

async function click(element: HTMLElement) {
  await act(async () => element.click());
}

beforeEach(() => {
  sent = [];
  closed = 0;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("a day-long hourly project with no date", () => {
  const project = projectOf({ step_hours: 1, step_count: 24, start_unix_s: null });

  it("offers the eight products, all ticked", async () => {
    await open(project);
    expect(products()).toHaveLength(8);
    expect(products().every((box) => box.checked)).toBe(true);
    const text = feature("products")?.textContent ?? "";
    expect(text).toContain("Copernicus MULTIOBS");
    expect(text).toContain("Copernicus DUACS");
    expect(text).toContain("Copernicus L4 wind");
    expect(text).toContain("ASCAT Metop-B/C");
    expect(text).toContain("CCMP NRT");
    expect(text).toContain("produced by Remote Sensing Systems");
    expect(text).toContain("Sea-surface temperature");
    expect(text).toContain("NOAA OISST");
    expect(text).toContain("NOAA Geo-Polar Blended");
    expect(text).toContain("OSTIA (Met Office)");
    expect(text).toContain("NOAA National Centers for Environmental Information");
    expect(text).toContain("from NOAA CoastWatch");
    // The attribution the licence asks for, as it words it.
    expect(text).toContain("Generated using E.U. Copernicus Marine Service Information");
  });

  it("is a dialog named for what it does", async () => {
    await open(project);
    expect(dialog().getAttribute("role")).toBe("dialog");
    expect(dialog().getAttribute("aria-label")).toBe("Near-real-time data");
  });

  it("will not import nothing", async () => {
    await open(project);
    expect(importButton().disabled).toBe(false);
    for (const box of products()) await click(box);
    expect(products().some((box) => box.checked)).toBe(false);
    expect(importButton().disabled).toBe(true);
    await click(importButton());
    expect(sent).toEqual([]);
  });

  /**
   * Three days, every product, the timeline dated and lengthened: the period
   * is 88 steps and the project has 24, so the box is there and ticked.
   */
  it("asks for three days of everything, dated and lengthened", async () => {
    await open(project);
    expect(feature("extend")).not.toBeNull();
    await click(importButton());
    expect(sent).toEqual([
      {
        products: ["multiobs", "duacs", "wind-l4", "ascat", "ccmp", "oisst", "geopolar", "ostia"],
        days: 3,
        set_start_time: true,
        extend_timeline: true,
      },
    ]);
  });

  /** The start is 29 September at midnight UTC, whatever zone the test runs in. */
  it("says where the period starts, in UTC", async () => {
    await open(project);
    expect(feature("days")?.textContent).toContain("From 2026-09-29 00:00 UTC to now.");
    expect(feature("set-start")?.textContent).toContain(
      "Set the timeline’s start to 2026-09-29 00:00 UTC",
    );
  });

  /**
   * Lengthened to 88 steps: 88 + 4 + 88 + 4 + 15 + 3 × 4 = 211 downloads
   * and 722.3, so 722 MB. Left at 24: 24 + 1 + 24 + 1 + 4 + 3 × 1 = 57 and
   * 195.5, so 196 MB — the steps past the timeline's end are not fetched,
   * so the cost has to follow the box.
   */
  it("prices what the timeline will show", async () => {
    await open(project);
    expect(dialog().textContent).toContain("211 downloads, about 722 MB on disk.");
    await click(checkbox("extend"));
    expect(dialog().textContent).toContain("57 downloads, about 196 MB on disk.");
    await click(importButton());
    expect(sent[0]?.extend_timeline).toBe(false);
  });

  it("sends only the products left ticked, in the import's order", async () => {
    await open(project);
    const [multiobs] = products();
    if (multiobs === undefined) throw new Error("expected a first product");
    await click(multiobs);
    await click(importButton());
    expect(sent[0]?.products).toEqual([
      "duacs",
      "wind-l4",
      "ascat",
      "ccmp",
      "oisst",
      "geopolar",
      "ostia",
    ]);
  });

  it("caps the days at what an hourly timeline holds", async () => {
    await open(project);
    const days = feature("days")?.querySelector<HTMLInputElement>('input[type="number"]');
    // Nine days is 232 hourly steps and ten is 256, past the 240 a timeline holds.
    expect(days?.max).toBe("9");
    expect(days?.min).toBe("1");
    expect(days?.value).toBe("3");
  });

  it("closes on Cancel and sends nothing", async () => {
    await open(project);
    const cancel = dialog().querySelector<HTMLButtonElement>(".modal-actions button");
    if (cancel === null) throw new Error("no Cancel button");
    await click(cancel);
    expect(closed).toBe(1);
    expect(sent).toEqual([]);
  });
});

describe("a project already long enough", () => {
  /** 240 steps hold the 88 the period needs: there is nothing to lengthen. */
  it("does not offer to lengthen, and does not ask to", async () => {
    await open(projectOf({ step_hours: 1, step_count: 240, start_unix_s: null }));
    expect(feature("extend")).toBeNull();
    await click(importButton());
    expect(sent).toHaveLength(1);
    expect(sent[0]?.extend_timeline).toBe(false);
    expect(sent[0]?.days).toBe(3);
  });
});

describe("a project that has a date", () => {
  /** 2020-01-01T00:00Z. The start is offered as a move, with what it is now. */
  it("offers to move the start, and says what it is", async () => {
    await open(projectOf({ step_hours: 1, step_count: 240, start_unix_s: 1_577_836_800 }));
    const text = feature("set-start")?.textContent ?? "";
    expect(text).toContain("Move the timeline’s start to 2026-09-29 00:00 UTC");
    expect(text).toContain("— it is 2020-01-01 00:00 now");
    expect(checkbox("set-start").checked).toBe(true);
    await click(checkbox("set-start"));
    await click(importButton());
    expect(sent[0]?.set_start_time).toBe(false);
  });
});
