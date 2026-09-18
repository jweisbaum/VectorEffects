import json, sys, re
def short(v, n):
    s = v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)
    s = re.sub(r"(Bearer )[A-Za-z0-9_-]+", r"\1***", s)
    return s if len(s) <= n else s[:n] + f" …[{len(s)} chars]"
n = int(sys.argv[2]) if len(sys.argv) > 2 else 400
for line in open(sys.argv[1]):
    try: e = json.loads(line)
    except Exception: continue
    t = e.get("type")
    if t == "system" and e.get("subtype") == "init":
        tools = e.get("tools", []); print("INIT tools:", len(tools), "| mcp:", [(m.get("name"), m.get("status")) for m in e.get("mcp_servers", [])])
    elif t == "assistant":
        for c in e["message"]["content"]:
            if c["type"] == "text" and c["text"].strip(): print("SAYS:", short(c["text"], n))
            elif c["type"] == "tool_use": print("CALL:", c["name"].replace("mcp__vectoreffects__", "ve."), short(c["input"], n))
    elif t == "user":
        for c in e["message"]["content"] if isinstance(e["message"]["content"], list) else []:
            if c.get("type") == "tool_result":
                body = c.get("content"); 
                if isinstance(body, list): body = " ".join(x.get("text", "[%s]" % x.get("type")) for x in body)
                print("  ->", ("ERROR " if c.get("is_error") else "") + short(body, n))
    elif t == "result":
        print("RESULT:", e.get("subtype"), "| turns", e.get("num_turns"), "| cost", round(e.get("total_cost_usd", 0), 3), "| denials", len(e.get("permission_denials", [])))
        print(short(e.get("result", ""), 3000))
