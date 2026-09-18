/**
 * Settings → MCP service (spec 8.8): the switch, the port, a button that
 * writes the service into Claude Code's or Codex's own configuration — and
 * the skill that says when to use it, where the client has skills — and the
 * same configuration as text for every other client.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import type { McpClient } from "../generated/McpClient";
import type { McpStatus } from "../generated/McpStatus";
import NumberField from "../NumberField";
import { api } from "../ipc";

/** The Claude Code command and a generic HTTP client entry. */
export function clientSnippets(status: McpStatus): { claudeCode: string; json: string; bridge: string } {
  const url = `http://127.0.0.1:${status.port}/mcp`;
  const auth = `Bearer ${status.token}`;
  return {
    claudeCode: `claude mcp add --scope user --transport http vectoreffects ${url} --header "Authorization: ${auth}"`,
    json: JSON.stringify({ mcpServers: { vectoreffects: { type: "http", url, headers: { Authorization: auth } } } }, null, 2),
    bridge: `npx -y mcp-remote ${url} --header "Authorization:${auth}"`,
  };
}

const CLIENT_NAMES: Readonly<Record<McpClient, string>> = {
  claude_code: "Claude Code",
  codex: "Codex",
  claude_desktop: "Claude Desktop",
};

/**
 * Claude Desktop is not given a configuration but an extension, which reads
 * the port and the token from the settings file as it goes. Nothing it holds
 * goes stale, and the installing is Claude Desktop's own dialog to finish: the
 * button can say it was opened there, never that it was added.
 */
const INSTALLS_ITSELF: ReadonlySet<McpClient> = new Set<McpClient>(["claude_desktop"]);

/**
 * What a client was last given, so the button can say when that has gone
 * stale: a rotated token or a changed port leaves the client holding a
 * configuration the listener no longer answers.
 */
function registrationKey(status: McpStatus): string {
  return `${status.port} ${status.token}`;
}

/** The button's words: not yet added, added as it stands, or added and since changed. */
export function registerLabel(client: McpClient, given: string | undefined, status: McpStatus): string {
  const name = CLIENT_NAMES[client];
  if (given === undefined) return `Add to ${name}`;
  if (INSTALLS_ITSELF.has(client)) return `Opened in ${name}`;
  return given === registrationKey(status) ? `Added to ${name}` : `Update in ${name}`;
}

export default function McpSection({ onError }: { onError: (err: unknown) => void }) {
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  // Remembered for as long as the dialog is open and no longer. The clients'
  // files are theirs and are not read back to find out: a reopened dialog
  // offers "Add" again, and adding twice is harmless.
  const [given, setGiven] = useState<Partial<Record<McpClient, string>>>({});
  const [adding, setAdding] = useState<McpClient | null>(null);
  // Where each client's skill was written. Claude Code reads its own; Claude
  // Desktop's is a zip only the person can upload, so the path is shown.
  const [skills, setSkills] = useState<Partial<Record<McpClient, string>>>({});

  // `onError` is `SettingsDialog`'s inline `report`, a new function identity
  // on every one of the dialog's re-renders (a shortcut rebind, a macro
  // deletion, a gradient load — nothing to do with this section). Reading it
  // through a ref, updated every render but never a dependency, keeps the
  // fetch-once effect below from re-running on someone else's state change.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => {
    void api
      .mcpStatus()
      .then(setStatus)
      .catch((err) => onErrorRef.current(err));
  }, []);

  const apply = useCallback((promise: Promise<McpStatus>) => {
    void promise.then(setStatus).catch((err) => onErrorRef.current(err));
  }, []);

  const copy = useCallback((label: string, text: string) => {
    void navigator.clipboard
      ?.writeText(text)
      .then(() => setCopied(label))
      .catch(() => setCopied(null));
  }, []);

  const register = useCallback((client: McpClient, current: McpStatus) => {
    setAdding(client);
    void api
      .registerMcpClient(client)
      .then((registered) => {
        setGiven((held) => ({ ...held, [client]: registrationKey(current) }));
        const skill = registered.skill;
        if (skill) setSkills((held) => ({ ...held, [client]: skill }));
      })
      .catch((err) => onErrorRef.current(err))
      .finally(() => setAdding(null));
  }, []);

  if (status === null)
    return (
      <section>
        <h3>MCP service</h3>
        <span>…</span>
      </section>
    );
  const snippets = clientSnippets(status);
  const url = `http://127.0.0.1:${status.port}/mcp`;

  return (
    <section>
      <h3>MCP service</h3>
      <p className="settings-note">
        Lets an AI client on this computer drive the application: open and edit projects, animate, import, export and take
        pictures of the map. Nothing outside this machine can reach it, and nothing can reach it while it is off.
      </p>
      <label className="settings-field settings-check">
        <input
          type="checkbox"
          checked={status.enabled}
          onChange={(event) => apply(api.setMcp(event.target.checked, status.port))}
        />
        Enable the MCP service on this computer
      </label>
      <label className="settings-field">
        Port
        <NumberField
          value={status.port}
          min={1}
          max={65535}
          step={1}
          commitWhileTyping={false}
          onCommit={(port) => apply(api.setMcp(status.enabled, Math.round(port)))}
        />
      </label>
      {status.bind_error && <p className="settings-error">{status.bind_error}</p>}
      {status.enabled && (
        <>
          <div className="settings-field">
            <span>URL</span>
            <code>{url}</code>
          </div>
          <div className="settings-field">
            <span>Token</span>
            <code>{status.token}</code>
            <button onClick={() => apply(api.rotateMcpToken())}>Rotate token</button>
          </div>
          <div className="settings-field">
            <span>
              {status.sessions === 0
                ? "No client connected"
                : `${status.sessions} client${status.sessions === 1 ? "" : "s"} connected${status.last_tool ? `, last: ${status.last_tool}` : ""}`}
            </span>
          </div>
          <div className="settings-field">
            {status.clients.map((client) => (
              <button key={client} disabled={adding !== null} onClick={() => register(client, status)}>
                {registerLabel(client, given[client], status)}
              </button>
            ))}
          </div>
          {status.clients.some((client) => !INSTALLS_ITSELF.has(client) && given[client] === registrationKey(status)) && (
            <p className="settings-note">Added. A session that is already running picks it up when it is restarted.</p>
          )}
          {skills.claude_code !== undefined && (
            <p className="settings-note">
              Claude Code was also given a skill that says when to use VectorEffects: <code>{skills.claude_code}</code>
            </p>
          )}
          {given.claude_desktop !== undefined && (
            <p className="settings-note">
              Claude Desktop is asking whether to install the VectorEffects extension; confirm it there. It needs
              installing once: a new token or port reaches it without another visit here.
            </p>
          )}
          {skills.claude_desktop !== undefined && (
            <p className="settings-note">
              A skill that says when to use VectorEffects was written to <code>{skills.claude_desktop}</code>. An
              extension cannot carry one, so add it yourself: in Claude Desktop's settings, under Skills, upload that
              file.
            </p>
          )}
          <details>
            <summary>Configuration for other clients</summary>
            <pre className="settings-snippet">{snippets.claudeCode}</pre>
            <button onClick={() => copy("claude", snippets.claudeCode)}>Copy Claude Code command</button>
            <pre className="settings-snippet">{snippets.json}</pre>
            <button onClick={() => copy("json", snippets.json)}>Copy HTTP client JSON</button>
            <pre className="settings-snippet">{snippets.bridge}</pre>
            <button onClick={() => copy("bridge", snippets.bridge)}>Copy stdio bridge command</button>
            {copied && <span className="settings-note">Copied.</span>}
          </details>
        </>
      )}
    </section>
  );
}
