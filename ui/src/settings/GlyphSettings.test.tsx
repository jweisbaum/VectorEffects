// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../generated/AppSettings";
import { DEFAULT_GLYPHS } from "../map/glyphAppearance";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const save = vi.hoisted(() => vi.fn());
vi.mock("../ipc", () => ({ api: { setGlyphAppearance: save } }));
import GlyphSettings from "./GlyphSettings";

let root: Root | undefined;
let container: HTMLDivElement;
const errors = vi.fn();
const initial = (): AppSettings => ({ glyphs: structuredClone(DEFAULT_GLYPHS) }) as AppSettings;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  save.mockReset();
  errors.mockReset();
});
async function render(settings = initial()) {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  function Harness() {
    const [value, setValue] = useState(settings);
    return <GlyphSettings settings={value} onSettings={setValue} onError={errors} />;
  }
  await act(async () => root!.render(<Harness />));
}
function input(label: string) {
  const element = container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`);
  if (!element) throw new Error(`Missing control ${label}`);
  return element;
}
async function number(label: string, value: string) {
  const element = input(label);
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => element.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
}

describe("glyph settings controls", () => {
  it("edits arrow size without changing barb settings and refreshes the sample", async () => {
    const saved = initial();
    saved.glyphs.arrow.size_percent = 175;
    save.mockResolvedValue(saved);
    await render();
    const before = container.querySelector('svg[aria-label="Arrow appearance preview"]')!.innerHTML;
    await number("Arrows Size (%)", "175");
    expect(save).toHaveBeenCalledWith("arrow", { property: "size_percent", value: 175 });
    expect(input("Wind barbs Size (%)").value).toBe("100");
    expect(input("Arrows Size (%)").value).toBe("175");
    expect(container.querySelector('svg[aria-label="Arrow appearance preview"]')!.innerHTML).not.toBe(before);
  });

  it("reveals shadow controls, sends signed offsets, and resets only that style", async () => {
    const settings = initial();
    settings.glyphs.barb.shadow.enabled = true;
    save.mockResolvedValue(settings);
    await render();
    expect(container.querySelector('input[aria-label="Wind barbs Shadow X (px)"]')).toBeNull();
    await act(async () => input("Wind barbs Drop shadow").click());
    expect(save).toHaveBeenLastCalledWith("barb", { property: "shadow_enabled", value: true });
    await number("Wind barbs Shadow X (px)", "-4");
    expect(save).toHaveBeenLastCalledWith("barb", { property: "shadow_offset_x_px", value: -4 });
    const reset = [...container.querySelectorAll("button")].find(button => button.textContent === "Reset wind barbs")!;
    await act(async () => reset.click());
    expect(save).toHaveBeenLastCalledWith("barb", { property: "reset" });
  });

  it("supports completely transparent glyphs and reports save failures", async () => {
    save.mockRejectedValue(new Error("Could not save preferences"));
    await render();
    await number("Wind barbs Opacity (%)", "0");
    expect(save).toHaveBeenCalledWith("barb", { property: "opacity_percent", value: 0 });
    expect(errors).toHaveBeenCalledWith(expect.objectContaining({ message: "Could not save preferences" }));
  });
});
