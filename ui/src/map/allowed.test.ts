/**
 * Which tools a layer will take (M51).
 *
 * The rule is the backend's, and the point of stating it here is that the
 * cursor can say it before the stroke rather than after. So what is asserted
 * is the *table*: every kind of layer against every kind of work, because a
 * gap in it is a tool that draws and then refuses.
 */
import { describe, expect, it } from "vitest";

import { layerTakes, type LayerSourceName, type ToolKindOfWork } from "./allowed";

const SOURCES: LayerSourceName[] = ["painted", "raster", "zarr", "image"];
const WORK: ToolKindOfWork[] = ["adds", "edits", "neither"];

describe("what a layer takes", () => {
  it("is answered for every layer and every kind of work", () => {
    for (const source of SOURCES) {
      for (const work of WORK) {
        expect(typeof layerTakes(source, work), `${source}/${work}`).toBe("boolean");
      }
    }
  });

  it("lets a painted layer take everything", () => {
    for (const work of WORK) expect(layerTakes("painted", work)).toBe(true);
  });

  /**
   * An imported field has one already, read from a file. A field painted over
   * it would be neither the file's nor the user's — but the eraser and the
   * modifiers act on what is there, which is exactly what they are for.
   */
  it("lets an imported field be edited but not painted on", () => {
    for (const source of ["raster", "zarr"] as const) {
      expect(layerTakes(source, "adds"), source).toBe(false);
      expect(layerTakes(source, "edits"), source).toBe(true);
    }
  });

  /** A history layer answers exactly as a forecast layer does. */
  it("treats a fetched history like an imported forecast", () => {
    for (const work of WORK) {
      expect(layerTakes("zarr", work)).toBe(layerTakes("raster", work));
    }
  });

  /** An image reaches no scene, so there is no field to add to or change. */
  it("lets an image take no field work at all", () => {
    expect(layerTakes("image", "adds")).toBe(false);
    expect(layerTakes("image", "edits")).toBe(false);
  });

  /** The hand, the selection and the measurements are nobody's business. */
  it("lets the tools that touch no field work anywhere", () => {
    for (const source of SOURCES) expect(layerTakes(source, "neither")).toBe(true);
  });
});
