// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import ThemePicker from "./ThemePicker";
import { THEMES } from "./themes";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement;
let root: Root;
const chosen = vi.fn();
function Example() {
  const [value, setValue] = useState("sage");
  return <><ThemePicker value={value} onChoose={id => { chosen(id); setValue(id); }} /><button>Next</button></>;
}
beforeEach(() => {
  chosen.mockClear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => root.render(<Example />));
});
afterEach(() => { act(() => root.unmount()); container.remove(); });
const trigger = () => container.querySelector<HTMLButtonElement>('[role="combobox"]')!;
const options = () => [...container.querySelectorAll<HTMLElement>('[role="option"]')];
const click = (node: HTMLElement) => act(() => node.click());
const key = (key: string) => act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
const colours = (node: Element) => [...node.querySelectorAll<HTMLElement>(".theme-palette > span")].map(swatch => swatch.style.backgroundColor);
const normalise = (hex: string) => { const el = document.createElement("span"); el.style.backgroundColor = hex; return el.style.backgroundColor; };

it("previews each theme's own palette and keeps the chosen palette visible after selection", () => {
  click(trigger());
  expect(options()).toHaveLength(THEMES.length);
  options().forEach((option, index) => {
    const roles = THEMES[index]!.roles;
    expect(colours(option)).toEqual([roles.bg, roles.active, roles.border, roles.accent, roles.text].map(normalise));
  });
  expect(options().filter(option => option.getAttribute("aria-selected") === "true").map(option => option.querySelector(".theme-name")?.textContent)).toEqual(["Sage & Teal"]);
  const paperColours = colours(options()[5]!);
  click(options()[5]!);
  expect(chosen).toHaveBeenCalledWith("paper");
  expect(options()).toHaveLength(0);
  expect(trigger().textContent).toContain("Paper");
  expect(colours(trigger())).toEqual(paperColours);
  expect(document.activeElement).toBe(trigger());
});

it("browses with arrow keys and cancels without changing the saved theme", () => {
  act(() => trigger().focus());
  key("ArrowDown");
  const active = () => document.getElementById(trigger().getAttribute("aria-activedescendant")!);
  expect(active()?.textContent).toContain("Sage & Teal");
  key("ArrowDown");
  expect(active()?.textContent).toContain("Ocean");
  key("ArrowUp");
  expect(active()?.textContent).toContain("Sage & Teal");
  key("End");
  expect(active()?.textContent).toContain("Paper");
  expect(chosen).not.toHaveBeenCalled();
  key("Escape");
  expect(options()).toHaveLength(0);
  expect(trigger().textContent).toContain("Sage & Teal");
  expect(document.activeElement).toBe(trigger());
  key(" ");
  key("Home");
  key("Enter");
  expect(chosen).toHaveBeenCalledWith("original");
  expect(trigger().textContent).toContain("Original (Midnight)");
});

it("dismisses on outside presses, Tab, and outside focus without selecting", () => {
  click(trigger());
  act(() => document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true })));
  expect(options()).toHaveLength(0);
  click(trigger());
  key("Tab");
  expect(options()).toHaveLength(0);
  click(trigger());
  act(() => container.querySelector<HTMLButtonElement>("button:last-child")!.focus());
  expect(options()).toHaveLength(0);
  expect(chosen).not.toHaveBeenCalled();
});

it("includes a saved Custom palette in the dropdown and keeps it selectable", () => {
  act(() => root.render(<ThemePicker value="ocean" custom={{ base: "sage", colours: { "roles.accent": "#ff1234" } }} onChoose={chosen} />));
  click(trigger());
  const custom = options().at(-1)!;
  expect(custom.textContent).toBe("Custom");
  expect(colours(custom)).toContain(normalise("#ff1234"));
  click(custom);
  expect(chosen).toHaveBeenCalledWith("custom");
});
