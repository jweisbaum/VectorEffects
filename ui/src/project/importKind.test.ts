import { describe, expect, it } from "vitest";

import { IMPORT_FILTERS, importOf } from "./importKind";

describe("what a chosen file imports as", () => {
  it("reads GRIB2 under each of its extensions, in any case", () => {
    for (const name of ["a.grib2", "a.grb2", "a.grib", "a.GRB"]) {
      expect(importOf(`/data/${name}`)).toEqual({ kind: "grib", path: `/data/${name}` });
    }
  });

  /** A store is a directory; the dialog picks the zarr.json at its top. */
  it("imports a routing store's directory from its zarr.json", () => {
    expect(importOf("/runs/routing_test/zarr.json")).toEqual({ kind: "zarr", path: "/runs/routing_test" });
    expect(importOf("C:\\runs\\week.zarr\\zarr.json")).toEqual({ kind: "zarr", path: "C:\\runs\\week.zarr" });
  });

  it("lays pictures as images, a GeoTIFF included", () => {
    for (const name of ["chart.png", "chart.jpg", "chart.JPEG", "chart.tif", "chart.tiff"]) {
      expect(importOf(`/maps/${name}`)?.kind).toBe("image");
    }
  });

  it("lays vector GIS files as GIS layers, other JSON included", () => {
    for (const name of ["coast.geojson", "zones.json", "coast.shp", "route.kml", "route.kmz", "track.gpx"]) {
      expect(importOf(`/gis/${name}`)?.kind).toBe("gis");
    }
  });

  it("refuses anything else rather than guessing", () => {
    expect(importOf("/data/notes.txt")).toBeNull();
    expect(importOf("/data/grib2")).toBeNull();
    expect(importOf("/data/zarr.json.bak")).toBeNull();
  });

  /** The dialog offers exactly what `importOf` reads, and the first filter is all of it. */
  it("filters the dialog to what it can read", () => {
    const [all, ...kinds] = IMPORT_FILTERS();
    const every = kinds.flatMap((filter) => filter.extensions);
    expect(new Set(all?.extensions)).toEqual(new Set(every));
    for (const extension of every) {
      const name = extension === "json" ? "zarr.json" : `f.${extension}`;
      expect(importOf(`/d/${name}`), extension).not.toBeNull();
    }
  });
});
