/**
 * What a file chosen through the layer panel's one Import button becomes.
 *
 * The four imports — GRIB2, a routing Zarr store, an image, GIS data — share
 * one native dialog, so the file's own name decides which it is. A Zarr
 * store is a directory, which a file dialog cannot also offer, so the store
 * is chosen by the `zarr.json` at its top and its directory is what imports.
 * A GeoTIFF is a picture, and lands as an image layer as it did from the GIS
 * button.
 */
import { t } from "../i18n";

export type ImportKind = "grib" | "zarr" | "image" | "gis";

const GRIB = ["grib2", "grb2", "grib", "grb"];
const IMAGE = ["tif", "tiff", "png", "jpg", "jpeg"];
const GIS = ["geojson", "json", "shp", "kml", "kmz", "gpx"];

/** The import a path is for, and the path to import; null for a format nothing reads. */
export function importOf(path: string): { kind: ImportKind; path: string } | null {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const name = path.slice(cut + 1);
  if (name.toLowerCase() === "zarr.json" && cut > 0) return { kind: "zarr", path: path.slice(0, cut) };
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return null;
  const extension = name.slice(dot + 1).toLowerCase();
  if (GRIB.includes(extension)) return { kind: "grib", path };
  if (IMAGE.includes(extension)) return { kind: "image", path };
  if (GIS.includes(extension)) return { kind: "gis", path };
  return null;
}

/**
 * The dialog's filters: everything first, so the dialog opens on every file
 * it can import, then each kind. A function, not a constant, so the names are
 * read in the language on screen.
 */
export function IMPORT_FILTERS(): { name: string; extensions: string[] }[] {
  const kinds = [
    { name: "GRIB2", extensions: GRIB },
    { name: t("Zarr store (zarr.json)"), extensions: ["json"] },
    { name: t("Image"), extensions: IMAGE },
    { name: t("GIS data"), extensions: GIS },
  ];
  const all = [...new Set(kinds.flatMap((kind) => kind.extensions))];
  return [{ name: t("Everything this imports"), extensions: all }, ...kinds];
}
