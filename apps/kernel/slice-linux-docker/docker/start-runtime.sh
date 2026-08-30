#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${CHARIOX_SLICE_ROOT:-/opt/chariox-slice}"
LOGS="$ROOT/logs"
KERNEL_PORT="${CHARIOX_SLICE_KERNEL_PORT:-43119}"
MCP_PORT="${CHARIOX_SLICE_MCP_PORT:-43120}"
CODEX_PORT_RANGE="${CHARIOX_SLICE_CODEX_PORT_RANGE:-43260-43279}"
OPENCODE_PORT_RANGE="${CHARIOX_SLICE_OPENCODE_PORT_RANGE:-43150-43169}"
PROVIDER_BIND_HOST="${CHARIOX_SLICE_PROVIDER_BIND_HOST:-127.0.0.1}"
RELAY_PORT="${CHARIOX_SLICE_RELAY_PORT:-43130}"
RELAY_URL="${CHARIOX_SLICE_RELAY_URL:-ws://127.0.0.1:$RELAY_PORT}"
RELAY_TOKEN="${CHARIOX_SLICE_RELAY_TOKEN:-slice-local}"
CLOUD_RELAY_CONFIG_JSON="${CHARIOX_SLICE_CLOUD_RELAY_CONFIG_JSON:-}"
CLOUD_RELAY_CONFIG_PATH="${CHARIOX_SLICE_CLOUD_RELAY_CONFIG_PATH:-}"
DAEMON_ALIAS="${CHARIOX_SLICE_DAEMON_ALIAS:-slice:linux}"
MACHINE_ID="${CHARIOX_SLICE_MACHINE_ID:-slice:linux}"
MACHINE_ALIAS="${CHARIOX_SLICE_MACHINE_ALIAS:-linux}"
CAPABILITY_ISOLATION_ROOT="${CHARIOX_SLICE_CAPABILITY_ISOLATION_ROOT:-$HOME/.chariox/managed-capabilities}"
mkdir -p "$LOGS"
mkdir -p "$CAPABILITY_ISOLATION_ROOT"
mkdir -p "$HOME/.chariox" /tmp/chariox-slice-state
mkdir -p "$HOME/.chariox/daemon"

wait_for_screen_session() {
  local session="$1"
  local label="$2"
  local attempt
  for attempt in $(seq 1 20); do
    if screen -ls | grep -E "[.]${session}[[:space:]]" >/dev/null; then
      return 0
    fi
    sleep 0.25
  done
  printf '[slice-runtime] %s failed to stay running in screen session %s\n' "$label" "$session" >&2
  screen -ls >&2 || true
  return 1
}

if [[ ! -f "$HOME/.chariox/config.toml" ]]; then
  cat >"$HOME/.chariox/config.toml" <<'EOF'
[state]
path = "/home/slice/.chariox/daemon/kernel.db"

[credential_vault]
backend = "process_memory"
EOF
fi

screen -S chariox-slice-relay -X quit >/dev/null 2>&1 || true
screen -S chariox-slice-kernel -X quit >/dev/null 2>&1 || true
screen -S chariox-slice-provider-bridge -X quit >/dev/null 2>&1 || true
# A restored container can retain the provider bridge briefly after its screen
# socket has already become stale. Kill only that orphan before rebinding the
# published provider ranges so the first restart is as reliable as a retry.
pkill -f "$ROOT/provider-port-bridge.mjs" >/dev/null 2>&1 || true

if [[ -z "$CLOUD_RELAY_CONFIG_JSON" && -n "$CLOUD_RELAY_CONFIG_PATH" && -f "$CLOUD_RELAY_CONFIG_PATH" ]]; then
  CLOUD_RELAY_CONFIG_JSON="$(cat "$CLOUD_RELAY_CONFIG_PATH")"
fi

if [[ -n "$CLOUD_RELAY_CONFIG_JSON" ]]; then
  printf '%s' "$CLOUD_RELAY_CONFIG_JSON" >"$HOME/.chariox/daemon/config.json"
  chmod 600 "$HOME/.chariox/daemon/config.json"
fi

PROVIDER_BRIDGE_READY_FILE="/tmp/chariox-slice-provider-bridge-ready.json"
PROVIDER_BRIDGE_LOG="$LOGS/provider-port-bridge.log"
rm -f "$PROVIDER_BRIDGE_READY_FILE"
: >"$PROVIDER_BRIDGE_LOG"
screen -L -Logfile "$PROVIDER_BRIDGE_LOG" -dmS chariox-slice-provider-bridge env \
  CHARIOX_SLICE_PROVIDER_BRIDGE_PORT_RANGES="$CODEX_PORT_RANGE,$OPENCODE_PORT_RANGE" \
  CHARIOX_SLICE_PROVIDER_BRIDGE_READY_FILE="$PROVIDER_BRIDGE_READY_FILE" \
  node "$ROOT/provider-port-bridge.mjs"
for attempt in $(seq 1 40); do
  if [[ -s "$PROVIDER_BRIDGE_READY_FILE" ]]; then
    break
  fi
  if ! screen -ls | grep -E '[.]chariox-slice-provider-bridge[[:space:]]' >/dev/null; then
    printf '[slice-runtime] provider bridge exited before becoming ready\n' >&2
    cat "$PROVIDER_BRIDGE_LOG" >&2 || true
    exit 1
  fi
  sleep 0.25
done
if [[ ! -s "$PROVIDER_BRIDGE_READY_FILE" ]]; then
  printf '[slice-runtime] provider bridge did not become ready\n' >&2
  cat "$PROVIDER_BRIDGE_LOG" >&2 || true
  exit 1
fi

if [[ -z "${CHARIOX_SLICE_RELAY_URL:-}" ]]; then
  screen -dmS chariox-slice-relay env CHARIOX_RELAY_HOST=0.0.0.0 CHARIOX_RELAY_PORT="$RELAY_PORT" CHARIOX_RELAY_TOKEN="$RELAY_TOKEN" "$ROOT/bin/chariox-relay"
  sleep 1
  wait_for_screen_session chariox-slice-relay relay
fi

kernel_relay_env=()
if [[ -n "$CLOUD_RELAY_CONFIG_JSON" ]]; then
  kernel_relay_env=()
else
  kernel_relay_env=(CHARIOX_RELAY_URL="$RELAY_URL" CHARIOX_RELAY_TOKEN="$RELAY_TOKEN")
fi

screen -dmS chariox-slice-kernel env \
  CHARIOX_KERNEL_PORT="$KERNEL_PORT" \
  CHARIOX_MCP_PORT="$MCP_PORT" \
  CHARIOX_CODEX_PORT_RANGE="$CODEX_PORT_RANGE" \
  CHARIOX_CODEX_BIND_HOST="$PROVIDER_BIND_HOST" \
  CHARIOX_OPENCODE_PORT_RANGE="$OPENCODE_PORT_RANGE" \
  CHARIOX_OPENCODE_BIND_HOST="$PROVIDER_BIND_HOST" \
  CHARIOX_DAEMON_ALIAS="$DAEMON_ALIAS" \
  CHARIOX_MACHINE_ID="$MACHINE_ID" \
  CHARIOX_MACHINE_ALIAS="$MACHINE_ALIAS" \
  CHARIOX_CAPABILITY_ISOLATION_ROOT="$CAPABILITY_ISOLATION_ROOT" \
  CHARIOX_BROWSER_CONTROLLER_SCRIPT="$ROOT/browser-controller.mjs" \
  CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT=1 \
  CHARIOX_OS_NAME="Linux slice" \
  "${kernel_relay_env[@]}" \
  CHARIOX_ACCEPT_REMOTE_LEASES=1 \
  "$ROOT/bin/chariox-kernel"

sleep 1
wait_for_screen_session chariox-slice-kernel kernel
screen -ls | sed -n '/chariox-slice-/p'
