// @vitest-environment happy-dom
import { afterEach, expect, it } from "vitest";
import { applyTheme, DEFAULT_THEME, mapColour, rgba, THEMES, themeOf } from "./themes";

afterEach(() => applyTheme(DEFAULT_THEME));

function contrast(a: string, b: string): number {
  const luminance = (hex: string) => {
    const channels = rgba(hex).slice(0, 3).map(c => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
    return channels[0]! * 0.2126 + channels[1]! * 0.7152 + channels[2]! * 0.0722;
  };
  const values = [luminance(a), luminance(b)].sort((x, y) => x - y);
  return (values[1]! + 0.05) / (values[0]! + 0.05);
}

it.each(THEMES)("$name keeps text and selected controls readable", theme => {
  const r = theme.roles;
  for (const [ink, background] of [[r.text, r.bg], [r.text, r.surface], [r.text, r.hover],
    [r.muted, r.surface], [r["selected-ink"], r["selected-bg"]], [r["selected-ink"], r["selected-hover"]]]) {
    expect(contrast(ink!, background!), `${theme.id}: ${ink} on ${background}`).toBeGreaterThanOrEqual(4.5);
  }
});

it("switches document and canvas colours together, including from light back to dark", () => {
  for (const id of ["paper", "original", "sage"]) {
    applyTheme(id);
    const theme = themeOf(id);
    expect(document.documentElement.dataset.theme).toBe(id);
    expect(document.documentElement.style.colorScheme).toBe(theme.scheme);
    expect(document.documentElement.style.getPropertyValue("--text")).toBe(theme.roles.text);
    expect(mapColour("source")).toBe(theme.map.source);
  }
  applyTheme("unknown");
  expect(document.documentElement.dataset.theme).toBe(DEFAULT_THEME);
});

it("resolves custom colours without changing presets and applies edits while Custom stays selected", () => {
  const custom = { base: "paper", colours: { "roles.bg": "#112233", "map.land": "#456789", "map.source": "#abcdef" } };
  const theme = themeOf("custom", custom);
  expect(theme.scheme).toBe("light");
  expect(theme.roles.bg).toBe("#112233");
  expect(theme.map.land).toBe("#456789");
  expect(theme.roles.text).toBe(themeOf("paper").roles.text);
  expect(themeOf("paper").roles.bg).toBe("#e9e7de");
  applyTheme("custom", custom);
  expect(document.documentElement.dataset.themeScheme).toBe("light");
  expect(mapColour("source")).toBe("#abcdef");
  applyTheme("custom", { ...custom, colours: { "map.source": "#123456" } });
  expect(mapColour("source")).toBe("#123456");
});
