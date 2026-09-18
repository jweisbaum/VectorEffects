"""Writes an MCP client configuration for the running VectorEffects service.

Usage: python3 make_config.py OUT.json

Copies the `vectoreffects` entry that Settings -> MCP service -> "Add to Claude
Code" registered at user scope, token included, into a file only this account
can read. The token is never printed.
"""
import json, os, sys

source = json.load(open(os.path.expanduser("~/.claude.json")))
server = source.get("mcpServers", {}).get("vectoreffects")
if not server or server.get("type") != "http":
    sys.exit("vectoreffects is not registered with Claude Code: turn the service on in Settings and press Add to Claude Code")
fd = os.open(sys.argv[1], os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
with os.fdopen(fd, "w") as out:
    json.dump({"mcpServers": {"vectoreffects": server}}, out)
print("wrote", sys.argv[1], "for", server["url"])
