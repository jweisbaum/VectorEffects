/**
 * The "save first?" decision, kept away from the components that render it.
 *
 * Creating or opening a project replaces the one that is open, and the backend
 * refuses to do that while it has unsaved changes unless it is told to discard
 * them (`unsaved-changes`). So the question has to be asked, and — more easily
 * got wrong — the answer has to be carried all the way to the call that
 * replaces the project. Two things go wrong otherwise: choosing Save and then
 * cancelling the file dialog falls through and discards the work anyway, or
 * "Don't save" throws the project away *before* the operation it was clearing
 * the way for turns out to be cancelled.
 */

/** What the user chose when told there are unsaved changes. */
export type UnsavedChoice = "save" | "discard" | "cancel";

/** Whether an operation may replace the open project, and on what terms. */
export type ReplaceDecision =
  | { proceed: false }
  /**
   * Go ahead. `discardUnsaved` is passed to the command that replaces the
   * project, so nothing is dropped until that command actually runs — cancel a
   * file dialog after this and the project is still there, unsaved.
   */
  | { proceed: true; discardUnsaved: boolean };

const STOP: ReplaceDecision = { proceed: false };
const GO: ReplaceDecision = { proceed: true, discardUnsaved: false };

/**
 * Decides whether an operation that replaces the open project may go ahead.
 *
 * `ask` is only called when there is something to lose. `save` reports whether
 * the save actually completed: a cancelled destination dialog or a failed write
 * both mean "no", and both stop the operation.
 */
export async function mayReplaceProject(
  project: { dirty: boolean } | null,
  ask: () => Promise<UnsavedChoice>,
  save: () => Promise<boolean>,
): Promise<ReplaceDecision> {
  if (project === null || !project.dirty) return GO;

  const choice = await ask();
  if (choice === "cancel") return STOP;
  if (choice === "discard") return { proceed: true, discardUnsaved: true };
  return (await save()) ? GO : STOP;
}
