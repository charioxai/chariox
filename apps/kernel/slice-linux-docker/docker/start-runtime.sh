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
RELAY_TOKEN_FILE="${CHARIOX_SLICE_RELAY_TOKEN_FILE:-}"
if [[ -n "$RELAY_TOKEN_FILE" ]]; then
  [[ -f "$RELAY_TOKEN_FILE" ]] || { printf '[slice-runtime] relay token file is missing\n' >&2; exit 1; }
  RELAY_TOKEN="$(cat "$RELAY_TOKEN_FILE")"
else
  RELAY_TOKEN="${CHARIOX_SLICE_RELAY_TOKEN:-slice-local}"
fi
CLOUD_RELAY_CONFIG_JSON="${CHARIOX_SLICE_CLOUD_RELAY_CONFIG_JSON:-}"
CLOUD_RELAY_CONFIG_PATH="${CHARIOX_SLICE_CLOUD_RELAY_CONFIG_PATH:-}"
DAEMON_ALIAS="${CHARIOX_SLICE_DAEMON_ALIAS:-slice:linux}"
MACHINE_ID="${CHARIOX_SLICE_MACHINE_ID:-slice:linux}"
MACHINE_ALIAS="${CHARIOX_SLICE_MACHINE_ALIAS:-linux}"
SLICE_ID="${CHARIOX_SLICE_ID:-}"
SLICE_OWNER_KERNEL_ID="${CHARIOX_SLICE_OWNER_KERNEL_ID:-}"
SLICE_OWNER_MACHINE_ID="${CHARIOX_SLICE_OWNER_MACHINE_ID:-}"
SLICE_OWNER_PUBLIC_KEY="${CHARIOX_SLICE_OWNER_PUBLIC_KEY:-}"
CAPABILITY_ISOLATION_ROOT="${CHARIOX_SLICE_CAPABILITY_ISOLATION_ROOT:-$HOME/.chariox/managed-capabilities}"
PROVIDER_HOME="${CHARIOX_MANAGED_PROVIDER_HOME:-$HOME/provider-home}"
PROVIDER_ISOLATION_PROBE="${CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE:-0}"
mkdir -p "$LOGS"
mkdir -p "$CAPABILITY_ISOLATION_ROOT"
mkdir -p "$HOME/.chariox" /tmp/chariox-slice-state
mkdir -p "$HOME/.chariox/daemon"
install -d -m 0700 "$PROVIDER_HOME" "$ROOT/private"

case "$PROVIDER_ISOLATION_PROBE" in
  0|1) ;;
  *) printf '[slice-runtime] managed provider isolation probe flag must be 0 or 1\n' >&2; exit 1 ;;
esac

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

KERNEL_LOCAL_AUTH_FILE="$ROOT/private/kernel-local-auth.token"
umask 077
dd if=/dev/urandom bs=48 count=1 status=none | base64 | tr -d '\n' >"$KERNEL_LOCAL_AUTH_FILE"
chmod 600 "$KERNEL_LOCAL_AUTH_FILE"

provider_probe_kernel_env=()
provider_probe_unselected="/tmp/chariox-managed-isolation-unselected-repository"
provider_probe_result="/workspace/.chariox-managed-isolation-probe.result"
if [[ "$PROVIDER_ISOLATION_PROBE" == "1" ]]; then
  real_codex="$(command -v codex)"
  [[ -x "$real_codex" ]] || { printf '[slice-runtime] real Codex executable is unavailable\n' >&2; exit 1; }
  mkdir -p "$provider_probe_unselected"
  provider_probe_kernel_env=(
    CHARIOX_CODEX_BIN="$ROOT/managed-provider-isolation-probe-wrapper.sh"
    CHARIOX_MANAGED_ISOLATION_REAL_PROVIDER="$real_codex"
    CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE="/workspace"
    CHARIOX_MANAGED_ISOLATION_PROBE_RESULT="$provider_probe_result"
    CHARIOX_MANAGED_ISOLATION_PROBE_UNSELECTED_REPOSITORY="$provider_probe_unselected"
  )
fi

KERNEL_LOCAL_AUTH_TOKEN="$(cat "$KERNEL_LOCAL_AUTH_FILE")"

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
  CHARIOX_SLICE_ID="$SLICE_ID" \
  CHARIOX_SLICE_OWNER_KERNEL_ID="$SLICE_OWNER_KERNEL_ID" \
  CHARIOX_SLICE_OWNER_MACHINE_ID="$SLICE_OWNER_MACHINE_ID" \
  CHARIOX_MANAGED_SLICE_RELAY_OWNER_PUBLIC_KEY="$SLICE_OWNER_PUBLIC_KEY" \
  CHARIOX_CAPABILITY_ISOLATION_ROOT="$CAPABILITY_ISOLATION_ROOT" \
  CHARIOX_MANAGED_PROVIDER_ISOLATION=1 \
  CHARIOX_MANAGED_PROVIDER_HOME="$PROVIDER_HOME" \
  CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE="$KERNEL_LOCAL_AUTH_FILE" \
  "${provider_probe_kernel_env[@]}" \
  CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT=1 \
  CHARIOX_OS_NAME="Linux slice" \
  "${kernel_relay_env[@]}" \
  CHARIOX_ACCEPT_REMOTE_LEASES=1 \
  "$ROOT/bin/chariox-kernel"

sleep 1
wait_for_screen_session chariox-slice-kernel kernel
[[ ! -e "$KERNEL_LOCAL_AUTH_FILE" ]] || { printf '[slice-runtime] kernel did not consume local auth token\n' >&2; exit 1; }
if [[ "$PROVIDER_ISOLATION_PROBE" == "1" ]]; then
  provider_probe_log="$LOGS/managed-provider-isolation-probe.log"
  if ! CHARIOX_KERNEL_URL="ws://127.0.0.1:$KERNEL_PORT" \
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN="$KERNEL_LOCAL_AUTH_TOKEN" \
    CHARIOX_PROBE_PACKAGE_JSON="/opt/chariox-toolchain/package.json" \
    node "$ROOT/managed-provider-isolation-probe.mjs" >"$provider_probe_log" 2>&1; then
    printf '[slice-runtime] managed provider isolation probe failed\n' >&2
    cat "$provider_probe_log" >&2
    screen -S chariox-slice-kernel -X quit >/dev/null 2>&1 || true
    unset KERNEL_LOCAL_AUTH_TOKEN
    rmdir "$provider_probe_unselected" >/dev/null 2>&1 || true
    exit 1
  fi
  cat "$provider_probe_log"
  rmdir "$provider_probe_unselected" >/dev/null 2>&1 || true
fi
unset KERNEL_LOCAL_AUTH_TOKEN
screen -ls | sed -n '/chariox-slice-/p'
