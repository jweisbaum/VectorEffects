import type { GlyphAppearance } from "../generated/GlyphAppearance";
import type { GlyphSettings } from "../generated/GlyphSettings";
import type { GlyphStyle } from "../generated/GlyphStyle";
import { GLYPH_SIZE_SCALE, GLYPH_TARGET_PX, mapGlyphLayout } from "./glyph";

/** Startup fallback, matching the persisted defaults served by Rust. */
export const DEFAULT_GLYPH_APPEARANCE: GlyphAppearance = {
  size_percent: 100, stroke_width_px: 1.8, color: "#f0f7ff", opacity_percent: 90,
  density_percent: 100, fade_with_speed: true,
  shadow: { enabled: false, color: "#000000", opacity_percent: 65, offset_x_px: 1.5, offset_y_px: 1.5 },
};
export const DEFAULT_GLYPHS: GlyphSettings = {
  arrow: { ...DEFAULT_GLYPH_APPEARANCE, shadow: { ...DEFAULT_GLYPH_APPEARANCE.shadow } },
  barb: { ...DEFAULT_GLYPH_APPEARANCE, shadow: { ...DEFAULT_GLYPH_APPEARANCE.shadow } },
};

/** Size and density are independent; a denser field must not shrink its marks. */
export function glyphDisplayLayout(style: GlyphStyle, pxPerDeg: number, pixelRatio: number, appearance: GlyphAppearance) {
  return {
    ...mapGlyphLayout(pxPerDeg, pixelRatio, appearance.density_percent),
    lengthPx: GLYPH_TARGET_PX[style] * GLYPH_SIZE_SCALE[style] * pixelRatio * appearance.size_percent / 100,
  };
}

/** Convert validated sRGB hex to the shader's RGB components. */
export function glyphRgb(color: string): [number, number, number] {
  return [parseInt(color.slice(1, 3), 16) / 255, parseInt(color.slice(3, 5), 16) / 255, parseInt(color.slice(5, 7), 16) / 255];
}

/** The same speed fade used by the map, including its calm visibility floor. */
export function glyphOpacity(appearance: GlyphAppearance, speedMps: number): number {
  return appearance.opacity_percent / 100 * (appearance.fade_with_speed ? Math.max(0.25, Math.min(1, speedMps / 25)) : 1);
}
