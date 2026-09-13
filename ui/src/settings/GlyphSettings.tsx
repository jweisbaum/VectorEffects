import { useRef } from "react";
import type { GlyphAppearance } from "../generated/GlyphAppearance";
import type { GlyphSetting } from "../generated/GlyphSetting";
import type { GlyphStyle } from "../generated/GlyphStyle";
import type { AppSettings } from "../generated/AppSettings";
import NumberField from "../NumberField";
import { api } from "../ipc";
import { glyphGeometry } from "../map/glyph";
import { DEFAULT_GLYPHS, glyphDisplayLayout } from "../map/glyphAppearance";

function GlyphSample({ style, appearance }: { style: GlyphStyle; appearance: GlyphAppearance }) {
  const { lengthPx } = glyphDisplayLayout(style, 1, 1, appearance);
  const spacing = 46 / Math.sqrt(appearance.density_percent / 100);
  const glyph = glyphGeometry(style, 65, 65, lengthPx, 40, appearance.stroke_width_px);
  const marks = Array.from({ length: Math.max(1, Math.floor(230 / spacing)) }, (_, i) => i * spacing + spacing / 2);
  const geometry = <>
    {glyph.lines.map((line, i) => <polyline key={`l${i}`} points={line.map(p => p.join(",")).join(" ")} fill="none" />)}
    {glyph.fills.map((line, i) => <polygon key={`p${i}`} points={line.map(p => p.join(",")).join(" ")} stroke="none" />)}
  </>;
  return <svg className="glyph-sample" viewBox="0 0 230 88" role="img" aria-label={`${style === "barb" ? "Wind barb" : "Arrow"} appearance preview`}>
    <g strokeWidth={appearance.stroke_width_px} strokeLinecap="round" strokeLinejoin="round">
      {appearance.shadow.enabled && marks.map(x => <g key={x}
        transform={`translate(${x + appearance.shadow.offset_x_px} ${56 + appearance.shadow.offset_y_px})`}
        stroke={appearance.shadow.color} fill={appearance.shadow.color}
        opacity={appearance.opacity_percent * appearance.shadow.opacity_percent / 10_000}>{geometry}</g>)}
      {marks.map(x => <g key={x} transform={`translate(${x} 56)`} stroke={appearance.color} fill={appearance.color}
        opacity={appearance.opacity_percent / 100}>{geometry}</g>)}
    </g>
  </svg>;
}

function StyleSettings({ style, value, change }: { style: GlyphStyle; value: GlyphAppearance; change: (setting: GlyphSetting) => void }) {
  const name = style === "barb" ? "Wind barbs" : "Arrows";
  const numeric = (label: string, current: number, min: number, max: number, step: number, commit: (value: number) => GlyphSetting) =>
    <label className="glyph-field">{label}<NumberField aria-label={`${name} ${label}`} value={current} min={min} max={max} step={step}
      commitWhileTyping={false} onCommit={number => change(commit(number))} /></label>;
  return <fieldset className="glyph-settings-card">
    <legend>{name}</legend>
    <GlyphSample style={style} appearance={value} />
    {numeric("Size (%)", value.size_percent, 25, 300, 5, value => ({ property: "size_percent", value: Math.round(value) }))}
    {numeric("Line width (px)", value.stroke_width_px, 0.5, 6, 0.1, value => ({ property: "stroke_width_px", value }))}
    <label className="glyph-field">Color<input aria-label={`${name} Color`} type="color" value={value.color}
      onChange={e => change({ property: "color", value: e.target.value })} /></label>
    {numeric("Opacity (%)", value.opacity_percent, 0, 100, 5, value => ({ property: "opacity_percent", value: Math.round(value) }))}
    {numeric("Density (%)", value.density_percent, 25, 300, 5, value => ({ property: "density_percent", value: Math.round(value) }))}
    <label className="glyph-check" title="Reduce opacity in slower flow while keeping calm wind visible"><input type="checkbox" checked={value.fade_with_speed}
      onChange={e => change({ property: "fade_with_speed", value: e.target.checked })} /> Fade with speed</label>
    <label className="glyph-check"><input type="checkbox" aria-label={`${name} Drop shadow`} checked={value.shadow.enabled}
      onChange={e => change({ property: "shadow_enabled", value: e.target.checked })} /> Drop shadow</label>
    {value.shadow.enabled && <div className="glyph-shadow-settings">
      <label className="glyph-field">Shadow color<input aria-label={`${name} Shadow color`} type="color" value={value.shadow.color}
        onChange={e => change({ property: "shadow_color", value: e.target.value })} /></label>
      {numeric("Shadow opacity (%)", value.shadow.opacity_percent, 0, 100, 5, value => ({ property: "shadow_opacity_percent", value: Math.round(value) }))}
      {numeric("Shadow X (px)", value.shadow.offset_x_px, -12, 12, 0.5, value => ({ property: "shadow_offset_x_px", value }))}
      {numeric("Shadow Y (px)", value.shadow.offset_y_px, -12, 12, 0.5, value => ({ property: "shadow_offset_y_px", value }))}
    </div>}
    <button type="button" onClick={() => change({ property: "reset" })}>Reset {name.toLowerCase()}</button>
  </fieldset>;
}

export default function GlyphSettings({ settings, onSettings, onError }: {
  settings: AppSettings; onSettings: (settings: AppSettings) => void; onError: (error: unknown) => void;
}) {
  const pending = useRef(Promise.resolve());
  const change = (style: GlyphStyle, setting: GlyphSetting) => {
    // Serialize saves from fast color-picker/number edits. Each command changes
    // one property, so even another control being edited cannot lose a setting.
    pending.current = pending.current.then(async () => {
      try { onSettings(await api.setGlyphAppearance(style, setting)); }
      catch (error) { onError(error); }
    });
  };
  const glyphs = settings.glyphs ?? DEFAULT_GLYPHS;
  return <section>
    <h3>Arrows &amp; wind barbs</h3>
    <p className="muted">Arrows show currents; barbs show wind. Density changes the number of marks, independently of their size.</p>
    <div className="glyph-settings-grid">
      {(["arrow", "barb"] as const).map(style => <StyleSettings key={style} style={style} value={glyphs[style]} change={setting => change(style, setting)} />)}
    </div>
  </section>;
}
