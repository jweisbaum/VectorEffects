// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const held = vi.hoisted(() => ({ set: false, sent: [] as string[], asked: 0 }));

vi.mock("../ipc", () => ({
  api: {
    earthdataStatus: () => {
      held.asked += 1;
      return Promise.resolve(held.set);
    },
    setEarthdataToken: (token: string) => {
      held.sent.push(token);
      held.set = token.trim() !== "";
      return Promise.resolve(held.set);
    },
  },
}));

const EarthdataSection = (await import("./EarthdataSection")).default;

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  held.set = false;
  held.sent.length = 0;
  held.asked = 0;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

async function render() {
  await act(async () => {
    root.render(<EarthdataSection onError={() => undefined} />);
  });
}

const field = () => host.querySelector<HTMLInputElement>('[data-feature="settings:earthdata-token"] input');
const button = (text: string) =>
  [...host.querySelectorAll("button")].find((b) => b.textContent === text) ?? null;

function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("the Earthdata token", () => {
  it("says no token is set, and saves a pasted one without showing it again", async () => {
    await render();
    expect(host.textContent).toContain("No token is set.");
    expect(button("Save token")?.disabled).toBe(true);
    expect(field()?.type).toBe("password");
    await act(async () => type(field()!, "eyJ0.secret"));
    await act(async () => button("Save token")?.click());
    expect(held.sent).toEqual(["eyJ0.secret"]);
    expect(host.textContent).toContain("A token is set.");
    expect(field()?.value).toBe("");
  });

  it("removes a token that is set", async () => {
    held.set = true;
    await render();
    expect(host.textContent).toContain("A token is set.");
    await act(async () => button("Remove token")?.click());
    expect(held.sent).toEqual([""]);
    expect(host.textContent).toContain("No token is set.");
    expect(button("Remove token")).toBeNull();
  });
});

/** The dialog hands a fresh error callback on every render; that is not a reason to ask again. */
it("asks whether a token is set once, however often the dialog renders", async () => {
  await render();
  await render();
  await render();
  expect(held.asked).toBe(1);
});
