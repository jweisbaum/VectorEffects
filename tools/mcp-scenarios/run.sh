#!/bin/bash
# run.sh NAME "PROMPT" [model]
#
# Hands one sentence to a fresh headless Claude Code agent that has the
# VectorEffects MCP service, web search, and nothing else: no project
# instructions, no plugins, no memory of this repository. What it does with
# the sentence is the test. The transcript lands in $VE_SCENARIO_DIR/NAME.jsonl;
# read it with show.py.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${VE_SCENARIO_DIR:?set VE_SCENARIO_DIR to a scratch directory outside the repository}"
NAME=$1; PROMPT=$2; MODEL=${3:-sonnet}
[ -f "$OUT/mcp.json" ] || python3 "$HERE/make_config.py" "$OUT/mcp.json"
# Its own empty working directory: the agent's relative paths and its idea of
# "here" must not be this repository.
mkdir -p "$OUT/work/$NAME" && cd "$OUT/work/$NAME"
claude -p "$PROMPT" --model "$MODEL" --setting-sources project,local \
  --mcp-config "$OUT/mcp.json" --strict-mcp-config \
  --tools "WebSearch,WebFetch,ToolSearch" \
  --allowedTools "mcp__vectoreffects,WebSearch,WebFetch,ToolSearch" \
  --no-session-persistence --output-format stream-json --verbose \
  > "$OUT/$NAME.jsonl" 2> "$OUT/$NAME.err" || true
python3 "$HERE/show.py" "$OUT/$NAME.jsonl" 300
