import { msg } from "../../i18n";
import type { Feature } from "../features";

/** The title bar, the status bar, the dock tabs and the start page. */
const features: Feature[] = [
  { id: "shell:help", label: msg("Help"), description: msg("Search features and the help reference."),
    keywords: [msg("search"), msg("find"), msg("manual")], topic: "workspace" },
  { id: "shell:language", label: msg("Language"), description: msg("Choose the language of the interface and help."),
    keywords: [msg("translation"), msg("locale")], topic: "settings" },
  { id: "start:settings", label: msg("Settings"), description: msg("Preferences for units, appearance, shortcuts and the macro library."),
    keywords: [msg("preferences")], topic: "settings" },

  // The title bar.
  { id: "shell:project-menu", label: msg("Project"), description: msg("Create, open, save and close projects."),
    keywords: [msg("file"), msg("menu")], topic: "projects" },
  { id: "project:new", label: msg("New…"), description: msg("Create a new project, asking first about unsaved changes."),
    keywords: [msg("new project"), msg("create")], topic: "projects", reveal: ["menu:project"] },
  { id: "project:open", label: msg("Open…"), description: msg("Open a saved project file."),
    keywords: [msg("open project"), msg("load")], topic: "projects", reveal: ["menu:project"] },
  { id: "project:save", label: msg("Save"), description: msg("Save the project to its file."),
    keywords: [msg("save project"), msg("store")], topic: "projects", reveal: ["menu:project"] },
  { id: "project:save-as", label: msg("Save As…"), description: msg("Save the project to a new file."),
    keywords: [msg("save a copy"), msg("duplicate file")], topic: "projects", reveal: ["menu:project"] },
  { id: "project:close", label: msg("Close"), description: msg("Close the project and return to the start page."),
    keywords: [msg("close project"), msg("start page")], topic: "projects", reveal: ["menu:project"] },
  { id: "shell:settings", label: msg("Settings"), description: msg("Preferences for units, appearance, shortcuts and the macro library."),
    keywords: [msg("preferences"), msg("options"), msg("gear")], topic: "settings" },
  { id: "shell:rename", label: msg("Project name"), description: msg("Click the project’s name in the title bar to rename it."),
    keywords: [msg("rename"), msg("title")], topic: "projects" },

  // The panels.
  { id: "dock:left", label: msg("Layer panel"), description: msg("Show or hide the layer panel on the left."),
    keywords: [msg("layers"), msg("sidebar"), msg("dock")], topic: "layers" },
  { id: "dock:right", label: msg("Properties and history panel"), description: msg("Show or hide the properties and history on the right."),
    keywords: [msg("inspector"), msg("sidebar"), msg("dock")], topic: "selection" },
  { id: "dock:bottom", label: msg("Timeline panel"), description: msg("Show or hide the timeline at the bottom."),
    keywords: [msg("timeline"), msg("keyframes"), msg("dock")], topic: "animation" },
  { id: "shell:properties", label: msg("Properties"), description: msg("Edit the selected objects’ options, positions and keys."),
    keywords: [msg("inspector"), msg("parameters"), msg("attributes")], topic: "selection", reveal: ["panel:right"] },
  { id: "shell:history", label: msg("History"), description: msg("The list of edits, for stepping back through undo and redo."),
    keywords: [msg("undo"), msg("redo"), msg("edits")], topic: "workspace", reveal: ["panel:right"] },

  // The status bar.
  { id: "shell:statusbar", label: msg("Status bar"), description: msg("Shows hints, errors, work in progress and the evaluator in use."),
    keywords: [msg("hint"), msg("error"), msg("progress")], topic: "workspace" },
  { id: "shell:export-grib", label: msg("Export GRIB…"), description: msg("Write the project’s field to a GRIB2 file."),
    keywords: [msg("export"), "GRIB2", msg("forecast file")], topic: "export" },
  { id: "shell:export-zarr", label: msg("Export Zarr…"), description: msg("Write wind and currents to a routing Zarr V3 store."),
    keywords: [msg("export"), msg("routing"), "Zarr V3"], topic: "export" },

  // The start page.
  { id: "start:new", label: msg("New project"), description: msg("Choose a name, resolution and time step, then create the project."),
    keywords: [msg("create"), msg("new project")], topic: "projects" },
  { id: "new:name", label: msg("Name"), description: msg("The new project’s name."),
    keywords: [msg("project name"), msg("title")], topic: "projects" },
  { id: "new:resolution", label: msg("Resolution"), description: msg("The grid spacing of the export. It cannot be changed later."),
    keywords: [msg("grid resolution"), msg("grid spacing"), msg("degrees")], topic: "projects" },
  { id: "new:time-step", label: msg("Time step"), description: msg("The hours between steps. It cannot be changed later."),
    keywords: [msg("interval"), msg("hours"), msg("frequency")], topic: "projects" },
  { id: "new:steps", label: msg("Steps"), description: msg("How many time steps the project has."),
    keywords: [msg("duration"), msg("frames"), msg("length")], topic: "projects" },
  { id: "start:browse", label: msg("Browse…"), description: msg("Open a saved project file."),
    keywords: [msg("open project"), msg("load")], topic: "projects" },
  { id: "start:open-grib", label: msg("Open from GRIB…"), description: msg("Create a project shaped by a GRIB2 file."),
    keywords: [msg("import"), "GRIB2", msg("forecast file")], topic: "imports" },
  { id: "start:open-zarr", label: msg("Open from Zarr…"), description: msg("Create a project from a routing Zarr directory."),
    keywords: [msg("import"), msg("routing"), "Zarr"], topic: "imports" },
  { id: "start:recent", label: msg("Recent projects"), description: msg("Reopen a project you opened before."),
    keywords: [msg("recent"), msg("last opened"), msg("history")], topic: "projects" },
  { id: "start:clear-recent", label: msg("Clear recent projects"), description: msg("Forget the list of recently opened projects."),
    keywords: [msg("forget"), msg("recent")], topic: "projects" },
  { id: "start:recover", label: msg("Recovered work"), description: msg("Reopen unsaved work after the application did not close cleanly."),
    keywords: [msg("autosave"), msg("recovery"), msg("crash")], topic: "projects" },
];
export default features;
