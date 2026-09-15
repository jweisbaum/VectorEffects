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
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ToolOptions from "./ToolOptions";
import NumberField from "../NumberField";
import ToolSelect from "../ToolSelect";
import { finishToolControl, focusMapForGesture } from "./focus";
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
      label: "Visible",
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

it("uses a pending numeric value on the first map press", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); const paint = vi.fn();
  function Host() {
    const [size, setSize] = useState(5);
    return <><NumberField value={size} onCommit={setSize} commitWhileTyping={false} min={1} max={20} />
      <canvas tabIndex={0} onPointerDownCapture={e => focusMapForGesture(e.currentTarget)} onPointerDown={() => paint(size)} /></>;
  }
  try {
    await act(async () => root.render(<Host />));
    const input = host.querySelector("input")!;
    await act(async () => {
      input.focus();
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "30");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => host.querySelector("canvas")!.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 })));
    expect(paint).toHaveBeenCalledTimes(1);
    expect(paint).toHaveBeenCalledWith(20);
    expect(document.activeElement).toBe(host.querySelector("canvas"));
  } finally { await act(async () => root.unmount()); host.remove(); }
});
it("releases a completed native control without stealing a newly focused field", () => {
  vi.useFakeTimers();
  const select = document.createElement("select"), input = document.createElement("input");
  document.body.append(select, input);
  try {
    select.focus(); finishToolControl({ target: select });
    expect(document.activeElement).not.toBe(select);
    input.focus(); vi.runAllTimers();
    expect(document.activeElement).toBe(input);
  } finally { select.remove(); input.remove(); vi.useRealTimers(); }
});

it("uses the first map drag after choosing an option without a native popup", async () => {
  const paint = vi.fn();
  function Host() {
    const [state, setState] = useState(STATE);
    return <><ToolOptions schema={SCHEMA} state={state} onChange={setState} convention="from" camera={CAMERA}
      picking={null} onPick={() => {}} sampling={false} onSample={() => {}} />
      <canvas tabIndex={0} onPointerDownCapture={e => focusMapForGesture(e.currentTarget)}
        onPointerDown={() => paint(state.values.EdgeMode)} /></>;
  }
  await act(async () => root.render(<Host />));
  const select = container.querySelector("select")!;
  const press = new PointerEvent("pointerdown", {bubbles:true, cancelable:true, button:0});
  await act(async () => select.dispatchEvent(press));
  expect(press.defaultPrevented, "the OS popup must not open").toBe(true);
  expect(document.querySelector('[role="listbox"]')).not.toBeNull();
  await act(async () => (document.querySelectorAll<HTMLButtonElement>('[role="option"]')[1]!).click());
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  await act(async () => container.querySelector("canvas")!.dispatchEvent(new PointerEvent("pointerdown", {bubbles:true, button:0})));
  expect(paint).toHaveBeenCalledTimes(1);
  expect(paint).toHaveBeenCalledWith({kind:"choice", index:1});
});

it("dismisses an open option list without consuming the first map press", async () => {
  const paint = vi.fn();
  await render(STATE);
  const canvas = document.createElement("canvas"); container.append(canvas);
  canvas.addEventListener("pointerdown", paint);
  await act(async () => container.querySelector("select")!.dispatchEvent(new PointerEvent("pointerdown", {bubbles:true, cancelable:true, button:0})));
  const press = new PointerEvent("pointerdown", {bubbles:true, cancelable:true, button:0});
  await act(async () => canvas.dispatchEvent(press));
  expect(press.defaultPrevented).toBe(false);
  expect(paint).toHaveBeenCalledTimes(1);
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});

it("supports keyboard choice and cancellation without opening a native menu", async () => {
  const changed = vi.fn();
  await act(async () => root.render(<ToolSelect defaultValue="round" onChange={e => changed(e.currentTarget.value)}>
    <option value="round">Round</option><option disabled value="disabled">Unavailable</option><option value="square">Square</option>
  </ToolSelect>));
  const select = container.querySelector("select")!;
  const key = async (value: string) => {
    const event = new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true });
    await act(async () => select.dispatchEvent(event));
    expect(event.defaultPrevented).toBe(true);
  };
  await key("ArrowDown");
  await key("Enter");
  expect(changed).toHaveBeenCalledWith("square");
  expect(document.activeElement).not.toBe(select);
  await key("Home");
  await key("Escape");
  expect(changed).toHaveBeenCalledTimes(1);
  expect(select.value).toBe("square");
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});
