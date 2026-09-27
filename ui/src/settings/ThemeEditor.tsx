import { useId, useState } from "react";
import type { CustomTheme } from "../generated/CustomTheme";
import { THEMES, themeName, themeOf, validColour, type Theme } from "./themes";
import { msg, useT } from "../i18n";

type ColourKey = `roles.${keyof Theme["roles"]}` | `map.${keyof Theme["map"]}` | `chart.${keyof Theme["chart"]}`;
const PRIMARY: [ColourKey, string][] = [
  ["roles.bg", msg("Window background")], ["roles.panel", msg("Panels")], ["roles.inset", msg("Controls")],
  ["roles.text", msg("Text")], ["roles.muted", msg("Secondary text")], ["roles.accent", msg("Accent")],
];
const GROUPS: [string, [ColourKey, string][]][] = [
  [msg("More interface colors"), [
    ["roles.surface", msg("Surface")], ["roles.raised", msg("Menus")], ["roles.hover", msg("Hover")],
    ["roles.active", msg("Active row")], ["roles.border", msg("Borders")], ["roles.border-subtle", msg("Divider lines")],
    ["roles.highlight", msg("Highlight")], ["roles.warning", msg("Warning text")], ["roles.error", msg("Error text")],
    ["roles.danger-border", msg("Danger border")], ["roles.danger-surface", msg("Danger background")],
    ["roles.selected-bg", msg("Selected button")], ["roles.selected-ink", msg("Selected button text")],
    ["roles.selected-border", msg("Selected button border")], ["roles.selected-hover", msg("Selected button hover")],
    ["roles.selection-ink", msg("Text selection")],
  ]],
  [msg("Map colors"), [
    ["map.sea", msg("Water")], ["map.land", msg("Land")], ["map.coast", msg("Coastlines")],
    ["map.graticule", msg("Grid lines")], ["map.void", msg("Outside the globe")],
    ["map.source", msg("Source outlines")], ["map.selection", msg("Selected outlines")],
    ["map.hover", msg("Brush hover")], ["map.ink", msg("Label background")],
    ["map.text", msg("Map text")], ["map.measure", msg("Measurements")],
  ]],
  [msg("Chart colors"), [
    ["chart.shallow", msg("Shallow water")], ["chart.middle", msg("Mid-depth water")],
    ["chart.deep", msg("Deep water")], ["chart.coast", msg("Chart coastlines")],
    ["chart.contour", msg("Contours")], ["chart.text", msg("Chart text")], ["chart.built", msg("Built-up areas")],
  ]],
];

/** Bundled panel backgrounds can have alpha; a custom colour picker edits RGB. */
function hexOf(colour: string): string {
  if (validColour(colour)) return colour;
  const rgb = colour.match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)/);
  return rgb ? `#${rgb.slice(1, 4).map(c => Number(c).toString(16).padStart(2, "0")).join("")}` : "#000000";
}
function colourOf(theme: Theme, key: ColourKey): string {
  const [group, role] = key.split(".") as ["roles" | "map" | "chart", string];
  return (theme[group] as Record<string, string>)[role]!;
}

export default function ThemeEditor({ initial, onSave, onCancel }: {
  initial: CustomTheme;
  onSave: (custom: CustomTheme) => Promise<void>;
  onCancel: () => void;
}) {
  const t = useT();
  const [draft, setDraft] = useState<CustomTheme>(() => ({ ...initial, colours: { ...initial.colours } }));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const base = themeOf(draft.base);
  const preview = themeOf("custom", draft);
  const valid = Object.values(draft.colours).every(colour => colour !== undefined && validColour(colour));
  const setColour = (key: ColourKey, value: string) => {
    setError(null);
    setDraft(before => ({ ...before, colours: { ...before.colours, [key]: value } }));
  };
  const fields = (rows: [ColourKey, string][]) => <div className="theme-colours">
    {rows.map(([key, label]) => {
      const value = draft.colours[key] ?? hexOf(colourOf(base, key));
      const swatch = validColour(value) ? value : hexOf(colourOf(base, key));
      return <div className="theme-colour" key={key}>
        <label htmlFor={`${id}-${key}`}>{t(label)}</label>
        <span className="theme-colour-swatch" style={{ backgroundColor: swatch }}>
          <input type="color" aria-label={t("{label} color", { label: t(label) })} value={swatch}
            onChange={event => setColour(key, event.target.value)} />
        </span>
        <input type="text" id={`${id}-${key}`} value={value} spellCheck={false} maxLength={7}
          aria-invalid={!validColour(value)} onChange={event => setColour(key, event.target.value.trim())} />
      </div>;
    })}
  </div>;

  return <fieldset className="theme-editor" disabled={saving}>
    <legend>{t("Custom theme")}</legend>
    <label className="settings-field">{t("Start from")}
      <select value={draft.base} onChange={event => {
        setDraft({ base: event.target.value, colours: {} }); setError(null);
      }}>
        {THEMES.map(theme => <option key={theme.id} value={theme.id}>{themeName(theme)}</option>)}
      </select>
    </label>
    <div className="theme-preview" style={{ background: preview.roles.bg, color: preview.roles.text, borderColor: preview.roles.border }}>
      <div style={{ background: preview.roles.panel, borderColor: preview.roles.border }}>
        <strong>{t("Theme preview")}</strong>
        <span style={{ color: preview.roles.muted }}>{t("Text and controls")}</span>
        <span className="theme-preview-accent" style={{ color: preview.roles.accent }}>{t("Accent")}</span>
        <span className="theme-preview-button" style={{ background: preview.roles["selected-bg"], color: preview.roles["selected-ink"] }}>{t("Selected")}</span>
      </div>
    </div>
    {fields(PRIMARY)}
    {GROUPS.map(([label, rows]) => <details key={label}>
      <summary>{t(label)}</summary>
      {fields(rows)}
    </details>)}
    {!valid && <p className="modal-error" role="alert">{t("Use six-digit hex colors, such as #87bba2.")}</p>}
    {error && <p className="modal-error" role="alert">{error}</p>}
    <div className="theme-editor-actions">
      <button type="button" onClick={() => { setDraft({ base: draft.base, colours: {} }); setError(null); }}>{t("Reset colors")}</button>
      <button type="button" onClick={onCancel}>{t("Cancel")}</button>
      <button type="button" className="primary" disabled={!valid || saving} onClick={async () => {
        setSaving(true); setError(null);
        try { await onSave(draft); }
        catch (error) { setError(String(error)); }
        finally { setSaving(false); }
      }}>{saving ? t("Saving…") : t("Save custom theme")}</button>
    </div>
  </fieldset>;
}
