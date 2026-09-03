import { describe, expect, it, vi } from "vitest";

import { mayReplaceProject } from "./saveGuard";

const never = () => {
  throw new Error("should not have been called");
};

describe("mayReplaceProject", () => {
  it("does not ask when there is nothing to lose", async () => {
    await expect(mayReplaceProject(null, never, never)).resolves.toEqual({
      proceed: true,
      discardUnsaved: false,
    });
    await expect(mayReplaceProject({ dirty: false }, never, never)).resolves.toEqual({
      proceed: true,
      discardUnsaved: false,
    });
  });

  it("stops when the user cancels", async () => {
    const save = vi.fn();
    await expect(
      mayReplaceProject({ dirty: true }, async () => "cancel", save),
    ).resolves.toEqual({ proceed: false });
    expect(save).not.toHaveBeenCalled();
  });

  // The discard is reported, not performed: the caller passes it to the command
  // that replaces the project, so a cancelled file dialog after this point
  // leaves the work where it was.
  it("reports a discard rather than acting on it", async () => {
    const save = vi.fn();
    await expect(
      mayReplaceProject({ dirty: true }, async () => "discard", save),
    ).resolves.toEqual({ proceed: true, discardUnsaved: true });
    expect(save).not.toHaveBeenCalled();
  });

  it("saves first when asked to, and then has nothing to discard", async () => {
    const save = vi.fn(async () => true);
    await expect(
      mayReplaceProject({ dirty: true }, async () => "save", save),
    ).resolves.toEqual({ proceed: true, discardUnsaved: false });
    expect(save).toHaveBeenCalledOnce();
  });

  // The failure this exists to prevent: choosing Save, cancelling the
  // destination dialog, and having the work thrown away regardless.
  it("abandons the operation when the save does not complete", async () => {
    await expect(
      mayReplaceProject({ dirty: true }, async () => "save", async () => false),
    ).resolves.toEqual({ proceed: false });
  });
});
