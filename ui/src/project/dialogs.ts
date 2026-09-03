/**
 * Native file dialogs.
 *
 * A web `<input type="file">` cannot give a real path, which a desktop app
 * needs in order to save back to the same file, so these go through Tauri's
 * dialog plugin.
 */
import { open, save } from "@tauri-apps/plugin-dialog";

import { EXTENSION } from "./format";

const FILTERS = [{ name: "VectorEffects project", extensions: [EXTENSION] }];

/** Asks for a project to open. Returns null if the user cancelled. */
export async function pickProjectToOpen(): Promise<string | null> {
  const chosen = await open({ multiple: false, directory: false, filters: FILTERS });
  return typeof chosen === "string" ? chosen : null;
}

/** Asks where to write a GRIB2 file. Returns null if the user cancelled. */
export async function pickGribDestination(projectName: string): Promise<string | null> {
  const chosen = await save({
    defaultPath: `${projectName}.grib2`,
    filters: [{ name: "GRIB2", extensions: ["grib2"] }],
  });
  return typeof chosen === "string" ? chosen : null;
}

/** Asks where to save. Returns null if the user cancelled. */
export async function pickProjectToSave(suggestedName: string): Promise<string | null> {
  const chosen = await save({
    defaultPath: `${suggestedName}.${EXTENSION}`,
    filters: FILTERS,
  });
  return typeof chosen === "string" ? chosen : null;
}
