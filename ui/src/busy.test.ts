import { describe, expect, it } from "vitest";

import { beginBusy, currentBusy, isBusy } from "./busy";

describe("the busy store", () => {
  it("spins while any task runs, and ending one twice is harmless", () => {
    expect(isBusy(currentBusy())).toBe(false);
    const endImport = beginBusy("Importing GRIB");
    const endSave = beginBusy("Saving");
    expect(currentBusy().labels).toEqual(["Importing GRIB", "Saving"]);
    endImport();
    endImport();
    expect(currentBusy().labels).toEqual(["Saving"]);
    expect(isBusy(currentBusy())).toBe(true);
    endSave();
    expect(isBusy(currentBusy())).toBe(false);
  });
});
