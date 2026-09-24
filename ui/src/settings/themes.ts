import catalogue from "./themes.json";
import type { CustomTheme } from "../generated/CustomTheme";

export const THEMES = catalogue;
export const DEFAULT_THEME = "sage";
export type Theme = typeof THEMES[number];
export type MapColour = keyof Theme["map"];
export const validColour = (value: string): boolean => /^#[0-9a-f]{6}$/i.test(value);
export function themeOf(id: string | undefined, custom?: CustomTheme | null): Theme {
  if (id === "custom" && custom) {
    const base = themeOf(custom.base === "custom" ? DEFAULT_THEME : custom.base);
    const theme = { ...base, id: "custom", name: "Custom", roles: { ...base.roles }, map: { ...base.map }, chart: { ...base.chart } };
    for (const [key, colour] of Object.entries(custom.colours)) {
      const [group, role] = key.split(".");
      if ((group === "roles" || group === "map" || group === "chart") && role && colour && validColour(colour)) {
        const colours = theme[group] as Record<string, string>;
        if (Object.hasOwn(colours, role)) colours[role] = colour;
      }
    }
    return theme;
  }
  return THEMES.find(theme => theme.id === id) ?? THEMES.find(theme => theme.id === DEFAULT_THEME)!;
}
let current = themeOf(DEFAULT_THEME);

/** Applied to the document, including portals and the start page. */
export function applyTheme(id: string | undefined, custom?: CustomTheme | null): void {
  current = themeOf(id, custom);
  const root = document.documentElement;
  root.dataset.theme = current.id;
  root.dataset.themeScheme = current.scheme;
  root.style.colorScheme = current.scheme;
  for (const [role, colour] of Object.entries(current.roles)) root.style.setProperty(`--${role}`, colour);
}

export function rgba(hex: string, alpha = 1): [number, number, number, number] {
  return [parseInt(hex.slice(1, 3), 16) / 255, parseInt(hex.slice(3, 5), 16) / 255, parseInt(hex.slice(5, 7), 16) / 255, alpha];
}

/** Canvas overlays use the active application theme just like DOM controls. */
export function mapColour(role: MapColour, alpha = 1): string {
  const hex = current.map[role];
  if (alpha === 1) return hex;
  const [r, g, b] = rgba(hex).map(channel => Math.round(channel * 255));
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
