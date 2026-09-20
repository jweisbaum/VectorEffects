/**
 * Native file dialogs.
 *
 * A web `<input type="file">` cannot give a real path, which a desktop app
 * needs in order to save back to the same file, so these go through Tauri's
 * dialog plugin.
 */
import { open, save } from "@tauri-apps/plugin-dialog";

import { whileChoosing } from "../busy";
import { EXTENSION } from "./format";

const FILTERS = [{ name: "VectorEffects project", extensions: [EXTENSION] }];

/** Asks for a project to open. Returns null if the user cancelled. */
export async function pickProjectToOpen(): Promise<string | null> {
  const chosen = await whileChoosing("Choosing a project", () =>
    open({ multiple: false, directory: false, filters: FILTERS }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Asks for a GRIB2 file to import as a layer. Returns null if cancelled. */
export async function pickGribToImport(): Promise<string | null> {
  const chosen = await whileChoosing("Choosing a GRIB file", () =>
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
  const chosen = await whileChoosing("Choosing a Zarr directory", () =>
    open({ multiple: false, directory: true, title: "Open Zarr directory" }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** The S-57 chart directory: an exchange set's root (spec.md 4.11). */
export async function pickChartDirectory(): Promise<string | null> {
  const chosen = await whileChoosing("Choosing a chart directory", () =>
    open({ multiple: false, directory: true, title: "Choose the ENC chart directory" }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/**
 * Picks a GIS file to lay under the field (spec.md 4.11).
 *
 * A georeferenced raster is offered here too, and routed to the image-layer
 * import: a GeoTIFF is a picture, and the application already places one.
 * Which of the two a file is, is a question its extension answers.
 */
export async function pickGisToImport(): Promise<string | null> {
  const chosen = await whileChoosing("Choosing a GIS file", () =>
    open({
      multiple: false,
      directory: false,
      filters: [
        {
          name: "GIS data",
          extensions: ["geojson", "json", "shp", "kml", "kmz", "tif", "tiff"],
        },
      ],
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Whether a GIS path is a raster, which imports as an image layer. */
export function isGeoRaster(path: string): boolean {
  return /\.(tif|tiff)$/i.test(path);
}

/**
 * Picks an image to lay under the field (spec.md 4.9, M18).
 *
 * The three formats there are pure-Rust decoders for. A GeoTIFF is a `.tif`
 * like any other, so it is not offered separately: whether a file carries a
 * georeference is something only the file knows.
 */
export async function pickImageToImport(): Promise<string | null> {
  const chosen = await whileChoosing("Choosing an image", () =>
    open({
      multiple: false,
      directory: false,
      filters: [{ name: "Image", extensions: ["tif", "tiff", "png", "jpg", "jpeg"] }],
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Asks where to write a GRIB2 file. Returns null if the user cancelled. */
export async function pickGribDestination(projectName: string): Promise<string | null> {
  const chosen = await whileChoosing("Choosing where to export", () =>
    save({
      defaultPath: `${projectName}.grib2`,
      filters: [{ name: "GRIB2", extensions: ["grib2"] }],
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Asks where to write a Zarr V3 directory. */
export async function pickZarrDestination(projectName: string): Promise<string | null> {
  const chosen = await whileChoosing("Choosing where to export", () =>
    save({
      defaultPath: `${projectName}.zarr`,
      filters: [{ name: "Zarr V3", extensions: ["zarr"] }],
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}

/** Asks where to save. Returns null if the user cancelled. */
export async function pickProjectToSave(suggestedName: string): Promise<string | null> {
  const chosen = await whileChoosing("Choosing where to save", () =>
    save({
      defaultPath: `${suggestedName}.${EXTENSION}`,
      filters: FILTERS,
    }),
  );
  return typeof chosen === "string" ? chosen : null;
}
