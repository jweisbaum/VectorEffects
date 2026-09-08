// @vitest-environment happy-dom
/**
 * A tool's options hand the keyboard back to the map (M46).
 *
 * The option bar sits over the map, and a control that keeps focus keeps the
 * keyboard with it: the arrows that nudge a selection go to the menu instead,
 * the tool shortcuts type into it, and on WebKit the click that dismisses a
 * native menu's popup is swallowed before it reaches the canvas — which is
 * what made the first map click after changing a setting do nothing.
 *
 * Rendered rather than asserted against the source, because what matters is
 * that focus has actually gone by the time the change has been handled.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import ToolOptions from "./ToolOptions";
import type { ToolSchema } from "../generated/ToolSchema";
import type { ToolState } from "./tools";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** A tool with one menu and one checkbox, which is all this is about. */
const SCHEMA = {
  tool: "brush",
  label: "Brush",
  shortcut: "b",
  preview: "field",
  options: [
    {
      property: "EdgeMode",
      label: "Edge",
      unit: "none",
      variants: ["Blend", "Replace"],
      default: { kind: "choice", index: 0 },
      interpolations: [],
      depends_on: [],
    },
    {
      property: "Enabled",
      label: "Enabled",
      unit: "none",
      variants: [],
      default: { kind: "bool", value: true },
      interpolations: [],
      depends_on: [],
    },
  ],
} as unknown as ToolSchema;

const CAMERA = { centerLon: 0, centerLat: 0, pxPerDeg: 4 };

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

async function render(state: ToolState) {
  await act(async () => {
    root.render(
      <ToolOptions
        schema={SCHEMA}
        state={state}
        onChange={() => undefined}
        convention="from"
        camera={CAMERA}
        picking={null}
        onPick={() => undefined}
        sampling={false}
        onSample={() => undefined}
      />,
    );
  });
}

const STATE: ToolState = {
  values: {
    EdgeMode: { kind: "choice", index: 0 },
    Enabled: { kind: "bool", value: true },
  },
  unit: "km",
} as unknown as ToolState;

describe("a tool's option controls", () => {
  it("gives focus up after a menu is used", async () => {
    await render(STATE);
    const select = container.querySelector("select");
    expect(select, "the schema's choice option renders as a menu").not.toBeNull();
    select?.focus();
    expect(document.activeElement).toBe(select);
    await act(async () => {
      (select as HTMLSelectElement).value = "1";
      select?.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(document.activeElement, "the keyboard goes back to the map").not.toBe(select);
  });

  it("gives focus up after a checkbox is used", async () => {
    await render(STATE);
    const box = container.querySelector('input[type="checkbox"]');
    expect(box).not.toBeNull();
    (box as HTMLInputElement).focus();
    expect(document.activeElement).toBe(box);
    await act(async () => {
      (box as HTMLInputElement).click();
    });
    expect(document.activeElement).not.toBe(box);
  });
});
