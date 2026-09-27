/**
 * Opening the help reference from inside the interface, optionally at a page.
 * The native menu's item and F1 reach `Help` on their own; this is for the
 * Help search, which opens a page it found.
 */

type Listener = (topic?: string) => void;
const listeners = new Set<Listener>();

export function openHelp(topic?: string): void {
  for (const listener of listeners) listener(topic);
}

export function onOpenHelp(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
