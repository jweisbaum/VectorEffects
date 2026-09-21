// @vitest-environment happy-dom
/**
 * `ImageControls`'s *Reset place* button (review finding 3(b)).
 *
 * The button used to show only for `image.georeferenced`. A hand-placed
 * image has no file georeference and so never showed it — which was fine
 * before control points existed, since the only way to move a hand-placed
 * image was a corner drag with its own undo. Now that the *only* way to move
 * one is a control point, a hand-placed image whose pairs have bent it badly
 * needs the same way back design.md §3 promises: "deleting every pair puts
 * the image back where it started". Undo does not survive a save and
 * reopen, so the button has to be reachable for this case too.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";

import type { ImageLayerView } from "../generated/ImageLayerView";
import { ImageControls } from "./LayerPanel";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** A minimal, fully-shaped view; each test overrides what it cares about. */
function view(overrides: Partial<ImageLayerView>): ImageLayerView {
  return {
    layer: 1,
    path: "/chart.png",
    loaded: true,
    width: 800,
    height: 600,
    placement: [0.001, 0, -70, 0, -0.001, 42],
    opacity: 1,
    control_points: [],
    warped: false,
    warp_mesh: [],
    warp_cells: 0,
    corners: [
      [-70, 42],
      [-69, 42],
      [-69, 41],
      [-70, 41],
    ],
    corner_residual_deg: [0, 0, 0, 0],
    georeferenced: false,
    ...overrides,
  };
}

async function render(image: ImageLayerView) {
  await act(async () =>
    root.render(
      <ImageControls image={image} onOpacity={() => {}} onReset={() => {}} onAlign={() => {}} />,
    ),
  );
}

function resetButton(): HTMLButtonElement | null {
  return (
    (Array.from(container.querySelectorAll("button")).find(
      (b) => b.textContent === "Reset place",
    ) as HTMLButtonElement | undefined) ?? null
  );
}

it("has no reset button for a hand-placed image with no control points", async () => {
  await render(view({ georeferenced: false, warped: false }));
  expect(resetButton()).toBeNull();
});

it("shows the reset button for a georeferenced image, as before", async () => {
  await render(view({ georeferenced: true, warped: false }));
  expect(resetButton()).not.toBeNull();
});

/** The regression this closes. */
it("shows the reset button for a hand-placed image once control points warp it", async () => {
  await render(view({ georeferenced: false, warped: true }));
  const button = resetButton();
  expect(button).not.toBeNull();
  // The title has to tell the truth about which case this is: there is no
  // file to go back to, only the pairs to clear.
  expect(button!.title).not.toContain("its own file");
});

it("still describes the georeferenced case as returning to the file", async () => {
  await render(view({ georeferenced: true, warped: false }));
  expect(resetButton()!.title).toContain("its own file");
});
