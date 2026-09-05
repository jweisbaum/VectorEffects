/**
 * Which panels are open (M25).
 *
 * The left sidebar, the right sidebar and the timeline dock each collapse to
 * a strip; inside the right one, properties and history fold separately. A
 * viewer's convenience, kept in `localStorage`, never a project's fact: two
 * people opening one file may want different panels put away.
 */

export interface PanelState {
  left: boolean;
  right: boolean;
  bottom: boolean;
  properties: boolean;
  history: boolean;
}

const KEY = "ve.panels";

export const OPEN: PanelState = {
  left: true,
  right: true,
  bottom: true,
  properties: true,
  history: true,
};

/** The remembered layout, or everything open. Storage may be absent or refuse. */
export function loadPanels(): PanelState {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (raw === null) return OPEN;
    return normalise(JSON.parse(raw) as Partial<PanelState>);
  } catch {
    return OPEN;
  }
}

/** Remembers a layout, if storage allows. */
export function savePanels(state: PanelState): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(state));
  } catch {
    // A private window, or storage refused: the layout lasts the session.
  }
}

/** The layout with one panel flipped. */
export function togglePanel(state: PanelState, panel: keyof PanelState): PanelState {
  return { ...state, [panel]: !state[panel] };
}

/** A stored layout with anything missing or malformed replaced by open. */
export function normalise(partial: Partial<PanelState> | null | undefined): PanelState {
  const pick = (key: keyof PanelState) =>
    typeof partial?.[key] === "boolean" ? (partial[key] as boolean) : true;
  return {
    left: pick("left"),
    right: pick("right"),
    bottom: pick("bottom"),
    properties: pick("properties"),
    history: pick("history"),
  };
}
