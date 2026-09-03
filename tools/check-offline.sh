#!/usr/bin/env bash
# Enforces invariant 5: the shipped application makes no network requests.
#
# Three layers, each checking what it can actually check:
#
#   1. Hand-written sources     -- any absolute URL is suspicious, so flag all.
#   2. The built bundle         -- only remote *resource references* are flagged.
#      A minified bundle is full of documentation URLs inside error strings
#      (React links to react.dev when it throws); scanning it for `http` finds
#      hundreds of those and nothing useful. What matters is whether the HTML or
#      CSS actually loads something remote.
#   3. The CSP                  -- the runtime enforcement. Even if a remote URL
#      slipped through, `default-src 'self'` stops the load. So the CSP itself
#      is checked for having been weakened.
#
# A socket-level test over the packaged app arrives with M10.
set -uo pipefail
cd "$(dirname "$0")/.."

CONF=crates/ve-app/tauri.conf.json

# Legitimately present, never fetched by the shipped app: the JSON schema
# reference, the dev server, and the Tauri IPC origin.
# `.localhost` is a reserved TLD (RFC 6761) that never resolves off-machine, so
# the Tauri IPC origin and our tile scheme's Windows form are local by definition.
ALLOW='schema\.tauri\.app|//localhost:|\.localhost|//127\.0\.0\.1'
# Pure comment lines. A URL in a comment fetches nothing, and the generated
# ts-rs bindings carry a provenance URL in their header.
COMMENT=':[0-9]+:[[:space:]]*(//|\*|/\*)'

fail=0
report() {
  echo "OFFLINE CHECK FAILED -- $1:"
  echo "$2" | sed 's/^/  /'
  fail=1
}

# --- 1. Hand-written sources -------------------------------------------------
hits=$(grep -rnE 'https?://' ui/src ui/index.html 2>/dev/null \
  | grep -vE "$COMMENT" | grep -vE "$ALLOW" || true)
[ -n "$hits" ] && report "absolute URL in frontend source" "$hits"

hits=$(find crates -name '*.rs' -exec grep -nHE 'https?://' {} + 2>/dev/null \
  | grep -vE "$COMMENT" | grep -vE "$ALLOW" || true)
[ -n "$hits" ] && report "absolute URL in rust source" "$hits"

hits=$(grep -nE 'https?://' "$CONF" 2>/dev/null | grep -vE "$ALLOW" || true)
[ -n "$hits" ] && report "absolute URL in tauri.conf.json" "$hits"

# --- 2. Built bundle: remote resource references only ------------------------
if [ -d ui/dist ]; then
  # <script src="http...">, <link href="http...">, and CSS url(http...)/@import.
  hits=$(grep -rnE '(src|href)[[:space:]]*=[[:space:]]*"https?://|url\([[:space:]]*["'"'"']?https?://|@import[^;]*https?://' \
    ui/dist 2>/dev/null | grep -vE "$ALLOW" || true)
  [ -n "$hits" ] && report "remote resource reference in built bundle" "$hits"

  # A websocket is the one network call that no CSP default-src catches loosely.
  hits=$(grep -rnoE 'wss?://[A-Za-z0-9.-]+' ui/dist 2>/dev/null | grep -vE "$ALLOW" || true)
  [ -n "$hits" ] && report "websocket URL in built bundle" "$hits"
fi

# --- 3. CSP: the actual runtime enforcement ----------------------------------
csp=$(grep -o '"csp"[^,]*' "$CONF" 2>/dev/null || true)
if [ -z "$csp" ]; then
  report "tauri.conf.json" "no CSP is declared"
elif ! echo "$csp" | grep -q "default-src 'self'"; then
  report "CSP" "default-src must be 'self', got: $csp"
elif echo "$csp" | grep -qE "\*|https://" ; then
  report "CSP" "allows a wildcard or remote origin: $csp"
fi

if [ "$fail" -eq 0 ]; then
  echo "offline check passed: no remote references, CSP is 'self'-only"
fi
exit "$fail"
