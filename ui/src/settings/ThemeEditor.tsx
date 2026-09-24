import { useId, useState } from "react";
import type { CustomTheme } from "../generated/CustomTheme";
import { THEMES, themeOf, validColour, type Theme } from "./themes";

type ColourKey = `roles.${keyof Theme["roles"]}` | `map.${keyof Theme["map"]}` | `chart.${keyof Theme["chart"]}`;
const PRIMARY: [ColourKey, string][] = [
  ["roles.bg", "Window background"], ["roles.panel", "Panels"], ["roles.inset", "Controls"],
  ["roles.text", "Text"], ["roles.muted", "Secondary text"], ["roles.accent", "Accent"],
];
const GROUPS: [string, [ColourKey, string][]][] = [
  ["More interface colors", [
    ["roles.surface", "Surface"], ["roles.raised", "Menus"], ["roles.hover", "Hover"],
    ["roles.active", "Active row"], ["roles.border", "Borders"], ["roles.border-subtle", "Dividers"],
    ["roles.highlight", "Highlight"], ["roles.warning", "Warning text"], ["roles.error", "Error text"],
    ["roles.danger-border", "Danger border"], ["roles.danger-surface", "Danger background"],
    ["roles.selected-bg", "Selected button"], ["roles.selected-ink", "Selected button text"],
    ["roles.selected-border", "Selected button border"], ["roles.selected-hover", "Selected button hover"],
    ["roles.selection-ink", "Text selection"],
  ]],
  ["Map colors", [
    ["map.sea", "Water"], ["map.land", "Land"], ["map.coast", "Coastlines"],
    ["map.graticule", "Grid lines"], ["map.void", "Outside the globe"],
    ["map.source", "Source outlines"], ["map.selection", "Selected outlines"],
    ["map.hover", "Brush hover"], ["map.ink", "Label background"],
    ["map.text", "Map text"], ["map.measure", "Measurements"],
  ]],
  ["Chart colors", [
    ["chart.shallow", "Shallow water"], ["chart.middle", "Mid-depth water"],
    ["chart.deep", "Deep water"], ["chart.coast", "Chart coastlines"],
    ["chart.contour", "Contours"], ["chart.text", "Chart text"], ["chart.built", "Built-up areas"],
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
        <label htmlFor={`${id}-${key}`}>{label}</label>
        <span className="theme-colour-swatch" style={{ backgroundColor: swatch }}>
          <input type="color" aria-label={`${label} color`} value={swatch}
            onChange={event => setColour(key, event.target.value)} />
        </span>
        <input type="text" id={`${id}-${key}`} value={value} spellCheck={false} maxLength={7}
          aria-invalid={!validColour(value)} onChange={event => setColour(key, event.target.value.trim())} />
      </div>;
    })}
  </div>;

  return <fieldset className="theme-editor" disabled={saving}>
    <legend>Custom theme</legend>
    <label className="settings-field">Start from
      <select value={draft.base} onChange={event => {
        setDraft({ base: event.target.value, colours: {} }); setError(null);
      }}>
        {THEMES.map(theme => <option key={theme.id} value={theme.id}>{theme.name}</option>)}
      </select>
    </label>
    <div className="theme-preview" style={{ background: preview.roles.bg, color: preview.roles.text, borderColor: preview.roles.border }}>
      <div style={{ background: preview.roles.panel, borderColor: preview.roles.border }}>
        <strong>Theme preview</strong>
        <span style={{ color: preview.roles.muted }}>Text and controls</span>
        <span className="theme-preview-accent" style={{ color: preview.roles.accent }}>Accent</span>
        <span className="theme-preview-button" style={{ background: preview.roles["selected-bg"], color: preview.roles["selected-ink"] }}>Selected</span>
      </div>
    </div>
    {fields(PRIMARY)}
    {GROUPS.map(([label, rows]) => <details key={label}>
      <summary>{label}</summary>
      {fields(rows)}
    </details>)}
    {!valid && <p className="modal-error" role="alert">Use six-digit hex colors, such as #87bba2.</p>}
    {error && <p className="modal-error" role="alert">{error}</p>}
    <div className="theme-editor-actions">
      <button type="button" onClick={() => { setDraft({ base: draft.base, colours: {} }); setError(null); }}>Reset colors</button>
      <button type="button" onClick={onCancel}>Cancel</button>
      <button type="button" className="primary" disabled={!valid || saving} onClick={async () => {
        setSaving(true); setError(null);
        try { await onSave(draft); }
        catch (error) { setError(String(error)); }
        finally { setSaving(false); }
      }}>{saving ? "Saving…" : "Save custom theme"}</button>
    </div>
  </fieldset>;
}
