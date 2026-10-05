import { msg } from "../../i18n";
import type { Feature } from "../features";

/**
 * The Settings dialog. `settings:` opens it (App); the second step brings the
 * section into view once the dialog is mounted (SettingsDialog).
 */
const open = (section: string) => ["settings:", `settings:${section}`];

const features: Feature[] = [
  { id: "settings:appearance", label: msg("Appearance"), description: msg("Theme, language and custom colours for the whole application."),
    keywords: [msg("look"), msg("colours"), msg("dark mode")], topic: "settings", reveal: open("appearance") },
  { id: "settings:theme", label: msg("Theme"), description: msg("Choose the colour theme of the application."),
    keywords: [msg("colours"), msg("dark mode"), msg("palette")], topic: "settings", reveal: open("appearance") },
  { id: "settings:custom-theme", label: msg("Custom theme"), description: msg("Start from a theme and change its colours one by one."),
    keywords: [msg("customize"), msg("colours"), msg("palette")], topic: "settings", reveal: open("appearance") },
  { id: "settings:language", label: msg("Language"), description: msg("Choose the language of the interface and help."),
    keywords: [msg("translation"), msg("locale")], topic: "settings", reveal: open("appearance") },

  { id: "settings:glyphs", label: msg("Arrows & wind barbs"), description: msg("How the arrows and wind barbs on the map look."),
    keywords: [msg("glyphs"), msg("symbols"), msg("appearance")], topic: "settings", reveal: open("glyphs") },
  { id: "settings:glyphs-arrow", label: msg("Arrows"), description: msg("Size, line width, colour and shadow of the current arrows."),
    keywords: [msg("glyphs"), msg("currents")], topic: "settings", reveal: open("glyphs") },
  { id: "settings:glyphs-barb", label: msg("Wind barbs"), description: msg("Size, line width, colour and shadow of the wind barbs."),
    keywords: [msg("glyphs"), msg("feathers")], topic: "settings", reveal: open("glyphs") },
  { id: "settings:glyph-density", label: msg("Density (%)"), description: msg("How many arrows or barbs are drawn, independently of their size."),
    keywords: [msg("spacing"), msg("glyphs"), msg("how many")], topic: "settings", reveal: open("glyphs") },
  { id: "settings:glyph-fade", label: msg("Fade with speed"), description: msg("Make arrows and barbs fainter in slower flow."),
    keywords: [msg("opacity"), msg("transparency")], topic: "settings", reveal: open("glyphs") },
  { id: "settings:glyph-shadow", label: msg("Drop shadow"), description: msg("Draw a shadow behind arrows and barbs for contrast."),
    keywords: [msg("contrast"), msg("outline")], topic: "settings", reveal: open("glyphs") },

  { id: "settings:shortcuts", label: msg("Shortcuts"), description: msg("Change the key that plays, steps, pans, zooms or picks up each tool."),
    keywords: [msg("keyboard"), msg("keys"), msg("hotkeys")], topic: "settings", reveal: open("shortcuts") },
  { id: "settings:shortcuts-reset", label: msg("Reset to defaults"), description: msg("Put every keyboard shortcut back as it was."),
    keywords: [msg("keyboard"), msg("restore")], topic: "settings", reveal: open("shortcuts") },

  { id: "settings:autosave", label: msg("Autosave"), description: msg("Keep a recovery snapshot, save into the project, or leave unsaved work until you save."),
    keywords: [msg("recovery"), msg("backup"), msg("crash")], topic: "projects", reveal: open("autosave") },

  { id: "settings:units", label: msg("Units"), description: msg("The units distances and speeds are shown and entered in."),
    keywords: [msg("knots"), msg("kilometres"), msg("nautical miles")], topic: "settings", reveal: open("units") },
  { id: "settings:distance-unit", label: msg("Distance"), description: msg("Show distances in kilometres or nautical miles."),
    keywords: [msg("nautical miles"), msg("kilometres")], topic: "settings", reveal: open("units") },
  { id: "settings:speed-unit", label: msg("Speed"), description: msg("Show speeds in knots, miles per hour or kilometres per hour."),
    keywords: [msg("knots"), msg("units")], topic: "settings", reveal: open("units") },
  { id: "settings:temperature-unit", label: msg("Temperature"), description: msg("Show sea-surface temperatures in degrees Celsius or Fahrenheit."),
    keywords: [msg("Celsius"), msg("Fahrenheit"), msg("sea-surface temperature")], topic: "settings", reveal: open("units") },

  { id: "settings:display", label: msg("Display"), description: msg("Colour scales and gradients the map paints speed with."),
    keywords: [msg("colour scale"), msg("gradient"), msg("legend")], topic: "settings", reveal: open("display") },
  { id: "settings:colour-scale", label: msg("Colour scale"), description: msg("The top speed of this project’s colour scale for wind and currents."),
    keywords: [msg("maximum speed"), msg("legend"), msg("range")], topic: "settings", reveal: open("display") },
  { id: "settings:gradient-wind", label: msg("This project’s colour gradient for wind"), description: msg("The colours the map paints wind speed with."),
    keywords: [msg("gradient"), msg("palette"), msg("colour map")], topic: "settings", reveal: open("display") },
  { id: "settings:gradient-current", label: msg("This project’s colour gradient for currents"), description: msg("The colours the map paints current speed with."),
    keywords: [msg("gradient"), msg("palette"), msg("colour map")], topic: "settings", reveal: open("display") },
  { id: "settings:default-scales", label: msg("Default colour scales"), description: msg("The colour scale new wind and current projects start with."),
    keywords: [msg("new project"), msg("colour scale")], topic: "settings", reveal: open("display") },

  { id: "settings:charts", label: msg("Charts"), description: msg("Electronic navigational charts drawn under the field."),
    keywords: [msg("nautical charts"), "ENC", "S-57"], topic: "view", reveal: open("charts") },
  { id: "settings:chart-directory", label: msg("Chart directory"), description: msg("Choose the folder an S-57 exchange set was unpacked into."),
    keywords: ["ENC", msg("folder")], topic: "view", reveal: open("charts") },

  { id: "settings:macros", label: msg("Macros"), description: msg("Where captured macros are kept, and how much space they use."),
    keywords: [msg("library")], topic: "macros", reveal: open("macros") },
  { id: "settings:macro-directory", label: msg("Library directory"), description: msg("The folder the macro library is kept in."),
    keywords: [msg("folder"), msg("macro library")], topic: "macros", reveal: open("macros") },
  { id: "settings:delete-macros", label: msg("Delete all macros"), description: msg("Empty the macro library. Projects keep their own copies."),
    keywords: [msg("clear"), msg("disk space"), msg("macro library")], topic: "macros", reveal: open("macros") },

  { id: "settings:earthdata", label: msg("NASA Earthdata"), description: msg("The token CMC sea-surface temperature is downloaded with."),
    keywords: [msg("login"), msg("credential"), "CMC", "PO.DAAC"], topic: "nrt", reveal: open("earthdata") },
  { id: "settings:earthdata-token", label: msg("Earthdata token"), description: msg("Paste, replace or remove the NASA Earthdata token."),
    keywords: [msg("login"), msg("credential"), msg("password")], topic: "nrt", reveal: open("earthdata") },
  { id: "settings:earthdata-status", label: msg("Earthdata token status"), description: msg("Whether a NASA Earthdata token is set."),
    keywords: [msg("login"), msg("credential")], topic: "nrt", reveal: open("earthdata") },

  { id: "settings:mcp", label: msg("MCP service"), description: msg("Let an AI client on this computer drive the application."),
    keywords: [msg("AI"), "Claude", msg("automation")], topic: "settings", reveal: open("mcp") },
  { id: "settings:mcp-enable", label: msg("Enable the MCP service on this computer"), description: msg("Turn the local MCP service on or off."),
    keywords: [msg("AI"), msg("automation")], topic: "settings", reveal: open("mcp") },
  { id: "settings:mcp-port", label: msg("Port"), description: msg("The local port the MCP service listens on."),
    keywords: ["MCP", msg("network")], topic: "settings", reveal: open("mcp") },
  { id: "settings:mcp-token", label: msg("Rotate token"), description: msg("Replace the MCP service’s token, revoking the old one."),
    keywords: ["MCP", msg("security"), msg("password")], topic: "settings", reveal: open("mcp") },
  { id: "settings:mcp-ask", label: msg("What an AI client may do without asking"), description: msg("Choose which MCP tools an AI client may use without asking you first."),
    keywords: ["MCP", msg("permission"), msg("approval"), msg("AI")], topic: "settings", reveal: open("mcp") },
  { id: "settings:mcp-clients", label: msg("Add to Claude Code, Codex or Claude Desktop"), description: msg("Write the MCP service into an AI client’s own configuration."),
    keywords: ["MCP", msg("connect"), msg("install")], topic: "settings", reveal: open("mcp") },
];
export default features;
