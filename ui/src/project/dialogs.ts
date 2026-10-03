/**
 * Native file dialogs.
 *
 * A web `<input type="file">` cannot give a real path, which a desktop app
 * needs in order to save back to the same file, so these go through Tauri's
 * dialog plugin.
 */
import { open, save } from "@tauri-apps/plugin-dialog";

import { whileChoosing } from "../busy";
import { msg, t } from "../i18n";
import { EXTENSION } from "./format";
import { IMPORT_FILTERS } from "./importKind";

/** A function, not a constant: the filter's name is read in the language on screen. */
const filters = () => [{ name: t("VectorEffects project"), extensions: [EXTENSION] }];

/** Asks for a project to open. Returns null if the user cancelled. */
export async function pickProjectToOpen(): Promise<string | null> {
  const chosen = await whileChoosing(msg("Choosing a project"), () =>
    open({ multiple: false, directory: false, filters: filters() }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Asks for a GRIB2 file to import as a layer. Returns null if cancelled. */
export async function pickGribToImport(): Promise<string | null> {
  const chosen = await whileChoosing(msg("Choosing a GRIB file"), () =>
    open({
      multiple: false,
      directory: false,
      filters: [{ name: "GRIB2", extensions: ["grib2", "grb2", "grib", "grb"] }],
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** A local Zarr store is a directory, even when its name has no extension. */
export async function pickZarrToImport(): Promise<string | null> {
  const chosen = await whileChoosing(msg("Choosing a Zarr directory"), () =>
    open({ multiple: false, directory: true, title: t("Open Zarr directory") }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** The S-57 chart directory: an exchange set's root (spec.md 4.11). */
export async function pickChartDirectory(): Promise<string | null> {
  const chosen = await whileChoosing(msg("Choosing a chart directory"), () =>
    open({ multiple: false, directory: true, title: t("Choose the ENC chart directory") }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/**
 * Picks a file for the layer panel's Import button: any format a layer can
 * come from, which `importOf` then sorts (spec.md 4.8). The first filter is
 * all of them, so the dialog opens showing everything it can import.
 */
export async function pickFileToImport(): Promise<string | null> {
  const chosen = await whileChoosing(msg("Choosing a file to import"), () =>
    open({ multiple: false, directory: false, filters: IMPORT_FILTERS() }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Asks where to write a GRIB2 file. Returns null if the user cancelled. */
export async function pickGribDestination(projectName: string): Promise<string | null> {
  const chosen = await whileChoosing(msg("Choosing where to export"), () =>
    save({
      defaultPath: `${projectName}.grib2`,
      filters: [{ name: "GRIB2", extensions: ["grib2"] }],
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Asks where to write a Zarr V3 directory. */
export async function pickZarrDestination(projectName: string): Promise<string | null> {
  const chosen = await whileChoosing(msg("Choosing where to export"), () =>
    save({
      defaultPath: `${projectName}.zarr`,
      filters: [{ name: "Zarr V3", extensions: ["zarr"] }],
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Asks where to save. Returns null if the user cancelled. */
export async function pickProjectToSave(suggestedName: string): Promise<string | null> {
  const chosen = await whileChoosing(msg("Choosing where to save"), () =>
    save({
      defaultPath: `${suggestedName}.${EXTENSION}`,
      filters: filters(),
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}
