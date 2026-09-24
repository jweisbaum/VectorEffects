// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import ThemeEditor from "./ThemeEditor";
import type { CustomTheme } from "../generated/CustomTheme";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement;
let root: Root;
const save = vi.fn();
const cancel = vi.fn();
beforeEach(() => {
  save.mockReset().mockResolvedValue(undefined); cancel.mockReset();
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });
const initial: CustomTheme = { base: "sage", colours: { "map.land": "#123456" } };
const render = () => act(() => root.render(<ThemeEditor initial={initial} onSave={save} onCancel={cancel} />));
const button = (name: string) => [...container.querySelectorAll("button")].find(button => button.textContent === name)!;
const input = (key: string) => [...container.querySelectorAll<HTMLInputElement>('input[type="text"]')].find(input => input.id.endsWith(key))!;
async function change(key: string, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(key), value);
    input(key).dispatchEvent(new Event("input", { bubbles: true }));
  });
}
const click = async (name: string) => { await act(async () => button(name).click()); };

it("previews edits locally and saves all edited interface and map colours together", async () => {
  render();
  await change("roles.bg", "#112233");
  await change("chart.deep", "#445566");
  expect(container.querySelector<HTMLElement>(".theme-preview")!.style.background).toBe("#112233");
  expect(save).not.toHaveBeenCalled();
  await click("Save custom theme");
  expect(save).toHaveBeenCalledWith({ base: "sage", colours: { "map.land": "#123456", "roles.bg": "#112233", "chart.deep": "#445566" } });
  expect(initial.colours).toEqual({ "map.land": "#123456" });
});

it("blocks malformed hex values and keeps the edited draft after a save failure", async () => {
  render();
  await change("roles.text", "#ff");
  expect(button("Save custom theme").disabled).toBe(true);
  expect(input("roles.text").getAttribute("aria-invalid")).toBe("true");
  await change("roles.text", "#ffffff");
  save.mockRejectedValueOnce(new Error("Preferences could not be written"));
  await click("Save custom theme");
  expect(container.textContent).toContain("Preferences could not be written");
  expect(input("roles.text").value).toBe("#ffffff");
  expect(button("Save custom theme").disabled).toBe(false);
  await click("Save custom theme");
  expect(save).toHaveBeenCalledTimes(2);
});

it("changes the base, resets colours, and cancels without saving", async () => {
  render();
  await change("roles.text", "#123456");
  await act(async () => {
    const select = container.querySelector("select")!;
    select.value = "paper"; select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  expect(input("roles.text").value).toBe("#243c43");
  expect(input("map.land").value).toBe("#d4dfcf");
  await change("roles.bg", "#123456");
  await click("Reset colors");
  expect(input("roles.bg").value).toBe("#e9e7de");
  await click("Cancel");
  expect(cancel).toHaveBeenCalledOnce();
  expect(save).not.toHaveBeenCalled();
});
