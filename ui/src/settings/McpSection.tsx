/** Native assistant connections, persistent verification and advanced local HTTP settings. */
import { useCallback, useEffect, useRef, useState } from "react";

import type { McpClient } from "../generated/McpClient";
import type { McpStatus } from "../generated/McpStatus";
import NumberField from "../NumberField";
import { api } from "../ipc";
import { t, useT } from "../i18n";

/** Advanced HTTP entry for clients that do not support native plugins. */
export function clientSnippets(status: McpStatus): { json: string } {
  const url = `http://127.0.0.1:${status.port}/mcp`;
  return { json: JSON.stringify({ mcpServers: { vectoreffects: { type: "http", url, headers: { Authorization: `Bearer ${status.token}` } } } }, null, 2) };
}

const CLIENT_NAMES: Readonly<Record<McpClient, string>> = {
  claude_code: "Claude Code",
  codex: "Codex",
  claude_desktop: "Claude Desktop",
};

/** How many clients are connected, and the last tool one called. */
function connectedLabel(sessions: number, lastTool: string | null): string {
  if (sessions === 0) return t("No client connected");
  if (lastTool) {
    return sessions === 1
      ? t("1 client connected, last: {tool}", { tool: lastTool })
      : t("{count} clients connected, last: {tool}", { count: sessions, tool: lastTool });
  }
  return sessions === 1 ? t("1 client connected") : t("{count} clients connected", { count: sessions });
}

export default function McpSection({ onError }: { onError: (err: unknown) => void }) {
  const t = useT();
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [connections, setConnections] = useState<import("../generated/McpConnection").McpConnection[]>([]);
  const [busy, setBusy] = useState<McpClient | null>(null);
  const [policy, setPolicy] = useState("workflow");
  const [tested, setTested] = useState(false);
  const [copied, setCopied] = useState(false);
  const onErrorRef = useRef(onError); onErrorRef.current = onError;
  const statusRef = useRef(status); statusRef.current = status;
  const serviceChanges = useRef<Promise<void>>(Promise.resolve());
  // A port commit and the next click can arrive before either IPC call completes.
  // Apply each change to the last saved state, so neither can undo the other.
  const changeService = (patch: { enabled?: boolean; port?: number }) => {
    const change = serviceChanges.current.then(async () => {
      const current = statusRef.current;
      if (!current) return;
      const next = await api.setMcp(patch.enabled ?? current.enabled, patch.port ?? current.port);
      statusRef.current = next;
      setStatus(next);
    });
    serviceChanges.current = change.catch(() => {});
    return change;
  };
  const refresh = useCallback(async () => {
    const [status, connections] = await Promise.all([api.mcpStatus(), api.mcpConnections()]);
    setStatus(status); setConnections(connections);
  }, []);
  useEffect(() => { void refresh().catch((error) => onErrorRef.current(error)); }, [refresh]);
  const act = async (client: McpClient, action: string) => {
    if (!status) return;
    setBusy(client); setTested(false);
    try {
      if (action === "connect") {
        if (!status.enabled) await changeService({ enabled: true });
        await api.registerMcpClient(client, policy);
      } else {
        await api.mcpConnectionAction(client, action);
        if (action === "test") setTested(true);
      }
      await refresh();
    } catch (error) { onErrorRef.current(error); }
    finally { setBusy(null); }
  };
  const labels: Record<string, string> = {
    connected: t("Connected"), restart_required: t("Restart required"),
    needs_repair: t("Needs repair"), unavailable: t("Unavailable"), disconnected: t("Not connected"),
  };
  return <section data-feature="settings:mcp">
    <h3>{t("AI assistants")}</h3>
    <p className="settings-note">{t("Connect once. Assistants can inspect, edit, calculate, download and export within your workflow.")}</p>
    {!status ? <span>…</span> : <>
      <label className="settings-field settings-check" data-feature="settings:mcp-enable">
        <input type="checkbox" checked={status.enabled} onChange={(event) => {
          void changeService({ enabled: event.target.checked }).catch(onErrorRef.current);
        }} />{t("Enable the MCP service on this computer")}
      </label>
      <label className="settings-field" data-feature="settings:mcp-clients">
        <span data-feature="settings:mcp-ask">{t("Permission policy")}</span>
        <select value={policy} onChange={(event) => setPolicy(event.target.value)}>
          <option value="workflow">{t("Routine work automatically; ask before irreversible actions")}</option>
          <option value="always_ask">{t("Ask before every action")}</option>
          <option value="unrestricted">{t("Never ask")}</option>
        </select>
      </label>
      {connections.map((connection) => <div className="settings-field" key={connection.client}>
        <strong>{CLIENT_NAMES[connection.client]}</strong>
        <span role="status">{labels[connection.state] ?? labels.disconnected!}</span>
        <button disabled={busy !== null || !connection.available} onClick={() => void act(connection.client, "connect")}>
          {connection.state === "disconnected" || connection.state === "unavailable" ? t("Connect") : t("Repair")}
        </button>
        <button disabled={busy !== null || !status.enabled} onClick={() => void act(connection.client, "test")}>{t("Test")}</button>
        {connection.state !== "disconnected" && connection.state !== "unavailable" && <button disabled={busy !== null} onClick={() => void act(connection.client, "disconnect")}>{t("Disconnect")}</button>}
      </div>)}
      <p className="settings-note">{t("Restart the assistant after connecting. Connected appears after its first successful tool call.")}</p>
      {tested && <p role="status">{t("The app is reachable. Ask the assistant to read its guide to verify the connection.")}</p>}
      {status.bind_error && <p className="settings-error">{status.bind_error}</p>}
      <details>
        <summary>{t("Configuration for other clients")}</summary>
        <span>{connectedLabel(status.sessions, status.last_tool)}</span>
        <label className="settings-field" data-feature="settings:mcp-port">{t("Port")}
          <NumberField value={status.port} min={1} max={65535} step={1} commitWhileTyping={false}
            onCommit={(port) => {void changeService({ port: Math.round(port) }).catch(onErrorRef.current);}} />
        </label>
        {status.enabled && <><pre className="settings-snippet">{clientSnippets(status).json}</pre><button onClick={() => { void navigator.clipboard.writeText(clientSnippets(status).json).then(() => setCopied(true)).catch(onErrorRef.current); }}>{t("Copy HTTP client JSON")}</button>{copied && <span role="status">{t("Copied.")}</span>}</>}
        <button data-feature="settings:mcp-token" onClick={() => {void api.rotateMcpToken().then(setStatus).catch(onErrorRef.current);}}>{t("Rotate token")}</button>
      </details>
    </>}
  </section>;
}
