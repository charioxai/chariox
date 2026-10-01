#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${CHARIOX_SLICE_ROOT:-/opt/chariox-slice}"
LOGS="$ROOT/logs"
if [[ -n "${CHARIOX_SLICE_PRIVATE_ROOT:-}" ]]; then
  LOGS="$CHARIOX_SLICE_PRIVATE_ROOT/runtime/logs"
fi
CODEX_PORT="${CHARIOX_SLICE_CODEX_PORT:-43252}"
OPENCODE_PORT="${CHARIOX_SLICE_OPENCODE_PORT:-43140}"
if [[ -n "${CHARIOX_SLICE_PRIVATE_ROOT:-}" ]]; then
  [[ -n "${CHARIOX_MANAGED_PROVIDER_HOME:-}" ]] || { printf '[slice-provider] protected provider home is missing\n' >&2; exit 1; }
  export HOME="$CHARIOX_MANAGED_PROVIDER_HOME"
fi
mkdir -p "$LOGS"

pkill -f "codex app-server.*$CODEX_PORT" >/dev/null 2>&1 || true
pkill -f "opencode serve.*$OPENCODE_PORT" >/dev/null 2>&1 || true

nohup codex app-server --listen "ws://127.0.0.1:$CODEX_PORT" >"$LOGS/codex-app-server.log" 2>&1 &
nohup opencode serve --hostname 127.0.0.1 --port "$OPENCODE_PORT" >"$LOGS/opencode-serve.log" 2>&1 &

sleep 3
pgrep -af "codex app-server|opencode serve" || true

