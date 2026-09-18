import json, sys, urllib.request
cfg = json.load(open(sys.argv[1]))["mcpServers"]["vectoreffects"]
def post(body, sid=None):
    h = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream", **cfg["headers"]}
    if sid: h["Mcp-Session-Id"] = sid
    r = urllib.request.urlopen(urllib.request.Request(cfg["url"], json.dumps(body).encode(), h))
    text = r.read().decode()
    data = [l[5:].strip() for l in text.splitlines() if l.startswith("data:") and l[5:].strip()] or [text]
    return r.headers.get("Mcp-Session-Id"), (json.loads(data[-1]) if data[-1].strip() else None)
sid, init = post({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"raw","version":"0"}}})
print("INSTRUCTIONS:", init["result"].get("instructions"))
try: post({"jsonrpc":"2.0","method":"notifications/initialized"}, sid)
except Exception as e: pass
_, tools = post({"jsonrpc":"2.0","id":2,"method":"tools/list"}, sid)
tools = tools["result"]["tools"]
json.dump(tools, open(sys.argv[2], "w"), indent=1)
for i, t in enumerate(tools):
    out = t.get("outputSchema"); flag = ""
    if out is not None and out.get("type") != "object": flag += f"  <-- outputSchema type={out.get('type')!r}"
    bad = [k for k, v in (t["inputSchema"].get("properties") or {}).items() if not isinstance(v, dict)]
    if bad: flag += f"  <-- non-object property schema: {bad} = {[t['inputSchema']['properties'][k] for k in bad]}"
    print(f"{i:2} {t['name']:20} {t.get('description','')[:150]}{flag}")
