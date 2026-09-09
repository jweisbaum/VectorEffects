/**
 * Dragging an object's lifetime window on the timeline (spec.md 9.4).
 *
 * The ordering, out of the component so it can be tested: the bug this exists
 * to prevent is a frame of the wrong thing on screen, which no type checker
 * sees and which is over before anyone can point at it.
 */

/**
 * Commits a drag, holding the preview until the document has caught up (M62).
 *
 * The order is the whole point. Clearing the preview first and *then* asking
 * the backend leaves the bar drawn from the document's old numbers for as long
 * as the round trip takes: the window snaps back to where it started and then
 * jumps to where it was dropped, which reads as the drop having failed and
 * been retried. The preview is what the user is looking at, so it stands until
 * there is something newer to draw.
 */
export async function commitRangeDrag<T>(
  write: () => Promise<T>,
  onDone: (value: T) => void,
  onError: (why: unknown) => void,
  clearPreview: () => void,
): Promise<void> {
  try {
    onDone(await write());
  } catch (why) {
    onError(why);
  } finally {
    clearPreview();
  }
}
