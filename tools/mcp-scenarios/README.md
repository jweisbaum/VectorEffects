# MCP scenarios

The MCP service's tests (`crates/ve-app/tests/mcp.rs`) say each tool does what
it says. They cannot say whether an agent handed a sentence reaches for the
right tool, which is the thing a person using the service finds out first.
These do: a fresh headless agent gets one sentence, the service and web search,
and its transcript is read against what it should have done.

They drive a **running application** with the service on and registered
(Settings -> MCP service -> Add to Claude Code), they reach the network, and
each costs a few tens of cents, so they are run by hand and not in CI.

```bash
export VE_SCENARIO_DIR=$(mktemp -d)     # never inside the repository
tools/mcp-scenarios/run.sh hurricane "Create a grib with the largest hurricane of 2024."
tools/mcp-scenarios/run.sh race      "Export a grib with the weather from the last newport bermuda race."
tools/mcp-scenarios/run.sh storm     "Create a tropical storm in the atlantic that goes from miami to nova scotia."
python3 tools/mcp-scenarios/check_storm.py "$VE_SCENARIO_DIR/mcp.json"   # after `storm`, project still open
```

| Sentence | What passing looks like |
|---|---|
| the largest hurricane of 2024 | Searches the web for the storm and its dates; `history_archives`, `project_new`, `import_history`, `export_grib` to an absolute path. **Never draws it.** Check the file: `grib_ls -p shortName,dataDate,stepRange FILE` starts on the storm's first day, and the wind around its centre turns counter-clockwise at hurricane strength. |
| the last Newport Bermuda Race | The same tools — and the *right year*, counted back from today's date, which leads the server's instructions for exactly this. |
| a tropical storm from Miami to Nova Scotia | `storm_create`, one call. `check_storm.py` reads the result back through `field_sample` and passes only a single `circle` object that turns counter-clockwise, starts at Miami, ends at Nova Scotia, starts at tropical-storm strength and **intensifies**. |

`list_tools.py CONFIG OUT.json` prints the instructions and every tool as a
client receives them, flagging any schema a strict client refuses.

**The agents write where they please.** One exported into `~/Downloads`, one
saved a project into the home folder, and before relative paths were refused
one wrote into `crates/ve-app`. Look for what a run left behind.

What the first runs found is in `plan.md` (2026-09-18) and as comments beside
each fix; `crates/ve-app/src/mcp/tools/guide.rs` is the instructions they led
to.
