"""Checks the open project against the storm scenario: one circle object, turning
counter-clockwise, that travels from Miami to Nova Scotia and intensifies.

Usage: python3 check_storm.py CONFIG.json   (make_config.py writes the config)

Reads the project back through the service itself: the object for where the
storm is, and field_sample for which way the evaluated wind turns and how
strong it is. Counter-clockwise is tested by what it means, not by the option.
"""
import json, math, sys, urllib.request
cfg = json.load(open(sys.argv[1]))["mcpServers"]["vectoreffects"]
sid = None
def rpc(method, params=None, notify=False):
    global sid
    h = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream", **cfg["headers"]}
    if sid: h["Mcp-Session-Id"] = sid
    body = {"jsonrpc": "2.0", "method": method}
    if params is not None: body["params"] = params
    if not notify: body["id"] = 1
    try: r = urllib.request.urlopen(urllib.request.Request(cfg["url"], json.dumps(body).encode(), h))
    except Exception:
        if notify: return None
        raise
    sid = r.headers.get("Mcp-Session-Id") or sid
    data = [l[5:].strip() for l in r.read().decode().splitlines() if l.startswith("data:") and l[5:].strip()]
    return json.loads(data[-1]) if data else None
def call(name, args=None):
    out = rpc("tools/call", {"name": name, "arguments": args or {}})["result"]
    if out.get("isError"): raise SystemExit(f"{name}: {out['content'][0]['text']}")
    return out["structuredContent"]
rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "check", "version": "0"}})
rpc("notifications/initialized", notify=True)

status = call("project_status")["project"]
assert status, "no project is open"
last = status["step_count"] - 1
layers = call("layers_list", {"step": 0})
objects = [o for l in layers["layers"] for o in l.get("objects", [])]
print(f"project: {status['name']!r}, {status['step_count']} steps x {status['step_hours']} h; objects: {[(o.get('name'), o.get('kind') or o.get('tool')) for o in objects]}")
storm = objects[-1]
def props(step): return {p["id"]: p["value"] for p in call("object_get", {"object": storm["id"], "step": step})["properties"]}
rows, ok = [], True
for step in sorted({0, last // 2, last}):
    p = props(step); lon, lat = p["Position"]["lon"], p["Position"]["lat"]
    radius_km = p["DiameterKm"]["value"] / 2
    # Four points at 40% of the radius: N, E, S, W of the centre.
    d = 0.4 * radius_km / 111.0; dx = d / math.cos(math.radians(lat))
    pts = [[lon, lat + d], [lon + dx, lat], [lon, lat - d], [lon - dx, lat]]
    s = call("field_sample", {"points": pts, "steps": [step], "kind": "wind"})["samples"]
    n, e, so, w = s
    # Counter-clockwise seen from above: north of centre the air moves west, east of it north, south of it east, west of it south.
    ccw = n["u_mps"] < 0 and e["v_mps"] > 0 and so["u_mps"] > 0 and w["v_mps"] < 0
    # Peak wind over a fine ring scan from the centre outward.
    scan = [[lon + dx * f / 0.4, lat] for f in (0.02, 0.1, 0.2, 0.3, 0.4, 0.6, 0.8, 0.95)]
    peak = max(x["speed_mps"] for x in call("field_sample", {"points": scan, "steps": [step], "kind": "wind"})["samples"] if x.get("defined", True))
    rows.append((step, lon, lat, p["DiameterKm"]["value"], peak, ccw))
    print(f"  step {step:3}: centre ({lat:6.2f}N, {lon:7.2f}E)  diameter {p['DiameterKm']['value']:6.0f} km  peak {peak:5.1f} m/s  counter-clockwise: {ccw}")
def near(lon, lat, lon0, lat0, deg): return abs(lon - lon0) < deg and abs(lat - lat0) < deg
checks = {
    "starts at Miami (25.8N 80.2W, within 2 deg)": near(rows[0][1], rows[0][2], -80.2, 25.8, 2),
    "ends at Nova Scotia (43-47N, 59.5-66.5W)": 43 <= rows[-1][2] <= 47.5 and -66.5 <= rows[-1][1] <= -59.5,
    "counter-clockwise at every sampled step": all(r[5] for r in rows),
    "tropical-storm strength at the start (peak under 33 m/s)": rows[0][4] < 33,
    "intensifies: peak wind rises along the track": rows[0][4] < rows[1][4] <= rows[-1][4] + 0.5 and rows[-1][4] > rows[0][4] + 2,
    "is a circle object": "circle" in json.dumps(storm).lower(),
}
for name, passed in checks.items(): print(("PASS " if passed else "FAIL ") + name)
sys.exit(0 if all(checks.values()) else 1)
