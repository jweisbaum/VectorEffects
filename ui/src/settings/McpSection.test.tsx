// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { McpStatus } from "../generated/McpStatus";
import type { McpConnection } from "../generated/McpConnection";
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const held = vi.hoisted(() => {
  const off: McpStatus = { enabled: false, port: 47391, token: "", bound_port: null, bind_error: null, sessions: 0, last_tool: null, clients: ["claude_code", "codex", "claude_desktop"], ask: "outside" };
  return {
    off, status: off,
    connections: [] as McpConnection[],
    mcpStatus: vi.fn(async () => held.status),
    mcpConnections: vi.fn(async () => held.connections),
    setMcp: vi.fn(async (enabled: boolean, port: number) => (held.status = { ...held.status, enabled, port, token: enabled ? "token-example" : "", bound_port: enabled ? port : null })),
    rotateMcpToken: vi.fn(async () => (held.status = { ...held.status, token: "rotated-example" })),
    registerMcpClient: vi.fn(async () => ({ skill: null })),
    mcpConnectionAction: vi.fn(async () => {}),
  };
});
vi.mock("../ipc", () => ({ api: held, IpcError: class extends Error {} }));
import McpSection from "./McpSection";
let root: Root;
let host: HTMLDivElement;
const buttons = (text: string) => Array.from(host.querySelectorAll("button")).filter(b => b.textContent === text);
async function render(onError = vi.fn()) { await act(async () => root.render(<McpSection onError={onError} />)); }
async function click(text: string, index = 0) { const button = buttons(text)[index]; expect(button).toBeDefined(); await act(async () => button!.click()); }
beforeEach(() => {
  vi.clearAllMocks();
  held.registerMcpClient.mockReset().mockResolvedValue({ skill: null });
  held.status = { ...held.off };
  held.connections = [
    { client: "codex", state: "disconnected", available: true, detail: "" },
    { client: "claude_code", state: "disconnected", available: true, detail: "" },
    { client: "claude_desktop", state: "unavailable", available: false, detail: "" },
  ];
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });
describe("AI assistant connections", () => {
  it("loads persistent state without installing or enabling anything", async () => {
    await render();
    expect(held.mcpConnections).toHaveBeenCalledTimes(1);
    expect(held.registerMcpClient).not.toHaveBeenCalled();
    expect(held.setMcp).not.toHaveBeenCalled();
    expect(host.textContent).toContain("Not connected");
    expect(host.textContent).not.toContain("http://127.0.0.1");
    expect(buttons("Connect")[2]?.disabled).toBe(true);
    await render();
    expect(held.mcpStatus).toHaveBeenCalledTimes(1);
  });
  it("connects once with routine-work policy and enables access first", async () => {
    await render(); await click("Connect");
    expect(held.setMcp).toHaveBeenCalledWith(true, 47391);
    expect(held.registerMcpClient).toHaveBeenCalledWith("codex", "workflow");
    expect(held.registerMcpClient).toHaveBeenCalledTimes(1);
    expect(held.setMcp.mock.invocationCallOrder[0]).toBeLessThan(held.registerMcpClient.mock.invocationCallOrder[0]!);
    expect(held.mcpConnections).toHaveBeenCalledTimes(2);
  });
  it("passes the selected policy only on an explicit connection action", async () => {
    await render();
    const select = host.querySelector("select")!;
    await act(async () => { select.value = "always_ask"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(held.registerMcpClient).not.toHaveBeenCalled();
    await click("Connect", 1);
    expect(held.registerMcpClient).toHaveBeenCalledWith("claude_code", "always_ask");
  });
  it("repairs an existing registration without claiming it is verified", async () => {
    held.status = { ...held.off, enabled: true };
    held.connections[0]!.state = "restart_required";
    await render();
    expect(host.textContent).toContain("Restart required");
    await click("Repair");
    expect(held.registerMcpClient).toHaveBeenCalledWith("codex", "workflow");
    expect(held.setMcp).not.toHaveBeenCalled();
  });
  it("tests app reachability and disconnects through their dedicated commands", async () => {
    held.status = { ...held.off, enabled: true };
    held.connections[0]!.state = "connected";
    await render(); await click("Test");
    expect(held.mcpConnectionAction).toHaveBeenCalledWith("codex", "test");
    expect(host.textContent).toContain("The app is reachable.");
    await click("Disconnect");
    expect(held.mcpConnectionAction).toHaveBeenCalledWith("codex", "disconnect");
    expect(held.registerMcpClient).not.toHaveBeenCalled();
  });
  it("rotates the token without requiring plugin reinstallation", async () => {
    held.status = { ...held.off, enabled: true, token: "token-example" };
    held.connections[0]!.state = "connected";
    await render(); await click("Rotate token");
    expect(held.rotateMcpToken).toHaveBeenCalledTimes(1);
    expect(held.registerMcpClient).not.toHaveBeenCalled();
    expect(host.textContent).toContain("rotated-example");
    expect(buttons("Repair")).toHaveLength(1);
  });
  it("keeps a committed port when enabling before its save completes", async () => {
    let finish!: (status: McpStatus) => void;
    held.setMcp.mockImplementationOnce(() => new Promise<McpStatus>(resolve => { finish = resolve; }));
    await render();
    const port = host.querySelector<HTMLInputElement>('[data-feature="settings:mcp-port"] input')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(port, "50321");
      port.dispatchEvent(new Event("input", { bubbles: true }));
      port.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => { port.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(held.setMcp).toHaveBeenCalledWith(false, 50321);
    await act(async () => { host.querySelector<HTMLInputElement>('[data-feature="settings:mcp-enable"] input')!.click(); });
    expect(held.setMcp).toHaveBeenCalledTimes(1);
    await act(async () => { finish({ ...held.off, port: 50321 }); });
    expect(held.setMcp).toHaveBeenLastCalledWith(true, 50321);
  });
  it("reports installation failure and leaves Connect available for retry", async () => {
    const error = new Error("Host unavailable");
    held.registerMcpClient.mockRejectedValue(error);
    const onError = vi.fn(); await render(onError); await click("Connect");
    expect(onError).toHaveBeenCalledWith(error);
    expect(buttons("Connect")[0]?.disabled).toBe(false);
    expect(host.textContent).toContain("Not connected");
  });
});
