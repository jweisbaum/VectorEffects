// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import Startup from "./startup";
import { applyTheme, DEFAULT_THEME, themeOf } from "./settings/themes";
import type { AppSettings } from "./generated/AppSettings";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const mocked = vi.hoisted(() => ({ appSettings: vi.fn(), show: vi.fn(), frontendLog: vi.fn() }));
vi.mock("./ipc", () => ({ api: mocked }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ show: mocked.show }) }));

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.clearAllMocks();
  mocked.show.mockResolvedValue(undefined);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  applyTheme(DEFAULT_THEME);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  applyTheme(DEFAULT_THEME);
});

it.each([
  { theme: "paper", custom_theme: null, background: themeOf("paper").roles.bg },
  { theme: "custom", custom_theme: { base: "paper", colours: { "roles.bg": "#123456" } }, background: "#123456" },
])("first renders and reveals the window in the saved $theme palette", async saved => {
  let finish!: (value: AppSettings) => void;
  const pending = new Promise<AppSettings>(resolve => { finish = resolve; });
  mocked.appSettings.mockReturnValue(pending);
  const settings = { theme: saved.theme, custom_theme: saved.custom_theme, language: "en" } as AppSettings;
  const screen = vi.fn((value: AppSettings | null) => {
    expect(value).toBe(settings);
    expect(document.documentElement.dataset.theme).toBe(saved.theme);
    expect(document.documentElement.style.getPropertyValue("--bg")).toBe(saved.background);
    return <main>Ready</main>;
  });
  mocked.show.mockImplementation(async () => {
    expect(host.textContent).toBe("Ready");
    expect(document.documentElement.style.getPropertyValue("--bg")).toBe(saved.background);
    expect(document.documentElement.style.colorScheme).toBe("light");
  });
  await act(async () => root.render(<StrictMode><Startup>{screen}</Startup></StrictMode>));
  expect(screen).not.toHaveBeenCalled();
  expect(mocked.show).not.toHaveBeenCalled();
  await act(async () => finish(settings));
  expect(screen).toHaveBeenCalled();
  expect(mocked.show).toHaveBeenCalled();
  expect(mocked.frontendLog).not.toHaveBeenCalled();
});

it("still reveals the app with defaults when preferences cannot be read", async () => {
  mocked.appSettings.mockRejectedValue(new Error("IPC unavailable"));
  await act(async () => root.render(<Startup>{settings => {
    expect(settings).toBeNull();
    expect(document.documentElement.dataset.theme).toBe(DEFAULT_THEME);
    return <main>Fallback</main>;
  }}</Startup>));
  expect(host.textContent).toBe("Fallback");
  expect(mocked.show).toHaveBeenCalledOnce();
});

it("ignores a settings response after startup unmounts", async () => {
  let finish!: (value: AppSettings) => void;
  mocked.appSettings.mockReturnValue(new Promise<AppSettings>(resolve => { finish = resolve; }));
  const screen = vi.fn(() => <main>Ready</main>);
  await act(async () => root.render(<Startup>{screen}</Startup>));
  await act(async () => root.render(null));
  await act(async () => finish({ theme: "paper", language: "en" } as AppSettings));
  expect(screen).not.toHaveBeenCalled();
  expect(mocked.show).not.toHaveBeenCalled();
  expect(document.documentElement.dataset.theme).toBe(DEFAULT_THEME);
});
