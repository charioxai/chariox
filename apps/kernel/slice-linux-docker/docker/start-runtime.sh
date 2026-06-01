#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${ARROBA_SLICE_ROOT:-/opt/arroba-slice}"
LOGS="$ROOT/logs"
KERNEL_PORT="${ARROBA_SLICE_KERNEL_PORT:-43119}"
MCP_PORT="${ARROBA_SLICE_MCP_PORT:-43120}"
CODEX_PORT_RANGE="${ARROBA_SLICE_CODEX_PORT_RANGE:-43260-43279}"
OPENCODE_PORT_RANGE="${ARROBA_SLICE_OPENCODE_PORT_RANGE:-43150-43169}"
PROVIDER_BIND_HOST="${ARROBA_SLICE_PROVIDER_BIND_HOST:-127.0.0.1}"
RELAY_PORT="${ARROBA_SLICE_RELAY_PORT:-43130}"
RELAY_URL="${ARROBA_SLICE_RELAY_URL:-ws://127.0.0.1:$RELAY_PORT}"
RELAY_TOKEN="${ARROBA_SLICE_RELAY_TOKEN:-slice-local}"
CLOUD_RELAY_CONFIG_JSON="${ARROBA_SLICE_CLOUD_RELAY_CONFIG_JSON:-}"
CLOUD_RELAY_CONFIG_PATH="${ARROBA_SLICE_CLOUD_RELAY_CONFIG_PATH:-}"
DAEMON_ALIAS="${ARROBA_SLICE_DAEMON_ALIAS:-slice:linux}"
MACHINE_ID="${ARROBA_SLICE_MACHINE_ID:-slice:linux}"
MACHINE_ALIAS="${ARROBA_SLICE_MACHINE_ALIAS:-linux}"
CAPABILITY_ISOLATION_ROOT="${ARROBA_SLICE_CAPABILITY_ISOLATION_ROOT:-$HOME/.arroba/managed-capabilities}"
mkdir -p "$LOGS"
mkdir -p "$CAPABILITY_ISOLATION_ROOT"
mkdir -p "$HOME/.arroba" /tmp/arroba-slice-state
mkdir -p "$HOME/.arroba/daemon"

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

if [[ ! -f "$HOME/.arroba/config.toml" ]]; then
  cat >"$HOME/.arroba/config.toml" <<'EOF'
[state]
path = "/tmp/arroba-slice-state/kernel.db"
EOF
fi

screen -S arroba-slice-relay -X quit >/dev/null 2>&1 || true
screen -S arroba-slice-kernel -X quit >/dev/null 2>&1 || true

if [[ -z "$CLOUD_RELAY_CONFIG_JSON" && -n "$CLOUD_RELAY_CONFIG_PATH" && -f "$CLOUD_RELAY_CONFIG_PATH" ]]; then
  CLOUD_RELAY_CONFIG_JSON="$(cat "$CLOUD_RELAY_CONFIG_PATH")"
fi

if [[ -n "$CLOUD_RELAY_CONFIG_JSON" ]]; then
  printf '%s' "$CLOUD_RELAY_CONFIG_JSON" >"$HOME/.arroba/daemon/config.json"
  chmod 600 "$HOME/.arroba/daemon/config.json"
fi

if [[ -z "${ARROBA_SLICE_RELAY_URL:-}" ]]; then
  screen -dmS arroba-slice-relay env ARROBA_RELAY_HOST=0.0.0.0 ARROBA_RELAY_PORT="$RELAY_PORT" ARROBA_RELAY_TOKEN="$RELAY_TOKEN" "$ROOT/bin/arroba-relay"
  sleep 1
  wait_for_screen_session arroba-slice-relay relay
fi

kernel_relay_env=()
if [[ -n "$CLOUD_RELAY_CONFIG_JSON" ]]; then
  kernel_relay_env=()
else
  kernel_relay_env=(ARROBA_RELAY_URL="$RELAY_URL" ARROBA_RELAY_TOKEN="$RELAY_TOKEN")
fi

screen -dmS arroba-slice-kernel env \
  ARROBA_KERNEL_PORT="$KERNEL_PORT" \
  ARROBA_MCP_PORT="$MCP_PORT" \
  ARROBA_CODEX_PORT_RANGE="$CODEX_PORT_RANGE" \
  ARROBA_CODEX_BIND_HOST="$PROVIDER_BIND_HOST" \
  ARROBA_OPENCODE_PORT_RANGE="$OPENCODE_PORT_RANGE" \
  ARROBA_OPENCODE_BIND_HOST="$PROVIDER_BIND_HOST" \
  ARROBA_DAEMON_ALIAS="$DAEMON_ALIAS" \
  ARROBA_MACHINE_ID="$MACHINE_ID" \
  ARROBA_MACHINE_ALIAS="$MACHINE_ALIAS" \
  ARROBA_CAPABILITY_ISOLATION_ROOT="$CAPABILITY_ISOLATION_ROOT" \
  ARROBA_OS_NAME="Linux slice" \
  "${kernel_relay_env[@]}" \
  ARROBA_ACCEPT_REMOTE_LEASES=1 \
  "$ROOT/bin/arroba-kernel"

sleep 1
wait_for_screen_session arroba-slice-kernel kernel
screen -ls | sed -n '/arroba-slice-/p'
