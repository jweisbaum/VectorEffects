// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import BetaGate from "./BetaGate";
import ConstantMotionDialog from "../timeline/ConstantMotionDialog";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const mocked = vi.hoisted(() => ({ betaStatus: vi.fn(), addConstantMotion: vi.fn() }));
vi.mock("../ipc", () => ({ api: mocked }));
let host: HTMLDivElement;
let root: Root;
beforeEach(() => { host = document.createElement("div"); document.body.append(host); root = createRoot(host); vi.clearAllMocks(); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
const click = async (label: string) => {
  const button = [...document.querySelectorAll("button")].find((b) => b.textContent === label);
  expect(button).toBeDefined();
  await act(async () => button!.click());
};

it("never mounts the editor before the clock check or after expiry", async () => {
  let finish: (status: string | null) => void = () => {};
  mocked.betaStatus.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  const child = vi.fn(() => <div>Editor is mounted</div>);
  const Child = child;
  await act(async () => root.render(<BetaGate><Child /></BetaGate>));
  expect(child).not.toHaveBeenCalled();
  const message = "Thank you for beta testing VectorEffects! The version you are using expired on January 1st, 2027. Please upgrade to use the latest version.";
  await act(async () => finish(message));
  expect(host.textContent).toContain(message);
  expect(child).not.toHaveBeenCalled();
  expect(host.querySelectorAll("button")).toHaveLength(0);
});

it("mounts the editor after an unexpired result", async () => {
  mocked.betaStatus.mockResolvedValue(null);
  await act(async () => root.render(<BetaGate><div>Editor is mounted</div></BetaGate>));
  expect(host.textContent).toBe("Editor is mounted");
});

it("asks before overwriting motion keys and submits the selected interval start", async () => {
  mocked.addConstantMotion.mockResolvedValue({ revision: 4 });
  const done = vi.fn();
  const close = vi.fn();
  await act(async () => root.render(<ConstantMotionDialog segment={{ object: 7, start: 3, end: 9, existing: 1 }} onDone={done} onClose={close} />));
  await click("Add motion");
  expect(mocked.addConstantMotion).not.toHaveBeenCalled();
  expect(document.body.textContent).toContain("Overwrite position keyframes?");
  await click("Overwrite and add motion");
  expect(mocked.addConstantMotion).toHaveBeenCalledWith(7, 3, 90, expect.closeTo(5.1444444, 5), true);
  expect(done).toHaveBeenCalledWith({ revision: 4 });
  expect(close).toHaveBeenCalledOnce();
});
