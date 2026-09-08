/**
 * The colour gradients the map paints speed with (spec.md 5.3, M42).
 *
 * The catalogue is the backend's — names, notes and stops in one table — and
 * this is the frontend's handle on it. Fetched once and remembered: it cannot
 * change while the application runs, and the map, the legend, the brush
 * previews and the settings dialog all want the same answer.
 */
import { api } from "./ipc";
import type { GradientView } from "./generated/GradientView";
import { RAMP_COLOURS, type Gradient } from "./map/ramp";

/** The gradient a project falls back to, matching `ve_core::colour`. */
export const DEFAULT_GRADIENT_ID = "vector";

let held: readonly GradientView[] | null = null;
let asked: Promise<readonly GradientView[]> | null = null;

/**
 * The catalogue, fetched at most once.
 *
 * A failure is not cached: an application that lost its gradients for the
 * rest of the session because one call failed at start-up would be worse
 * than one that asks again.
 */
export function loadGradients(): Promise<readonly GradientView[]> {
  if (held) return Promise.resolve(held);
  asked ??= api
    .colourGradients()
    .then((list) => {
      held = list;
      return list;
    })
    .catch((err: unknown) => {
      asked = null;
      throw err;
    });
  return asked;
}

/** What has been fetched so far, for a caller that cannot wait. */
export function knownGradients(): readonly GradientView[] {
  return held ?? [];
}

/**
 * The stops an identifier names.
 *
 * Falls back to the default, and then to the built-in copy, so a caller
 * always has a gradient to draw with: an empty catalogue is what the first
 * frames see, and a project may name a gradient a later version added.
 */
export function stopsOf(catalogue: readonly GradientView[], id: string): Gradient {
  const found =
    catalogue.find((entry) => entry.id === id) ??
    catalogue.find((entry) => entry.id === DEFAULT_GRADIENT_ID);
  if (!found) return RAMP_COLOURS;
  return found.stops as Gradient;
}
