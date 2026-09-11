import { describe, expect, it } from "vitest";

import { beginBusy, currentBusy, isBusy, whileChoosing } from "./busy";

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

describe("the wait while a dialog is up", () => {
  /**
   * The report (M74): clicking Open gave no sign that anything had happened.
   * The command's own marker starts after the decision prompt and the native
   * dialog, so the spinner answered the click only once a file had been
   * chosen. The dialog is part of the operation.
   */
  it("is busy while choosing and clear afterwards", async () => {
    expect(isBusy(currentBusy())).toBe(false);
    let seen = false;
    const chosen = await whileChoosing("Choosing a project", async () => {
      seen = isBusy(currentBusy());
      return "/tmp/a.veproj";
    });
    expect(seen, "the spinner was on while the dialog was up").toBe(true);
    expect(chosen).toBe("/tmp/a.veproj");
    expect(isBusy(currentBusy())).toBe(false);
  });

  /** A cancelled dialog is the common case and must not leave it spinning. */
  it("clears when the dialog is cancelled", async () => {
    await whileChoosing("Choosing a project", async () => null);
    expect(isBusy(currentBusy())).toBe(false);
  });

  /** Nor a dialog that fails, or the spinner turns for the rest of the session. */
  it("clears when the dialog throws", async () => {
    await expect(
      whileChoosing("Choosing a project", () => Promise.reject(new Error("no"))),
    ).rejects.toThrow("no");
    expect(isBusy(currentBusy())).toBe(false);
  });

  /** It names what is happening, which is what the spinner's tooltip shows. */
  it("says what it is waiting for", async () => {
    let label: readonly string[] = [];
    await whileChoosing("Choosing a GRIB file", async () => {
      label = currentBusy().labels;
      return null;
    });
    expect(label).toEqual(["Choosing a GRIB file"]);
  });
});
