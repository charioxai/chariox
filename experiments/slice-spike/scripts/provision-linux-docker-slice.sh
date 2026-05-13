#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

SLICE_NAME="${ARROBA_SLICE_NAME:-arroba-slice-linux-spike}"
SLICE_IMAGE="${ARROBA_SLICE_DOCKER_IMAGE:-arroba-slice-linux-spike:local}"
SLICE_HOME_VOLUME="${ARROBA_SLICE_HOME_VOLUME:-${SLICE_NAME}-home}"
SLICE_WORKSPACE="${ARROBA_SLICE_WORKSPACE:-$REPO_ROOT}"
SLICE_RECREATE="${ARROBA_SLICE_RECREATE:-0}"
SLICE_START_DESKTOP="${ARROBA_SLICE_START_DESKTOP:-1}"
SLICE_START_PROVIDER_SERVERS="${ARROBA_SLICE_START_PROVIDER_SERVERS:-1}"
SLICE_START_RUNTIME="${ARROBA_SLICE_START_RUNTIME:-0}"
SLICE_IMPORT_PROVIDER_AUTH="${ARROBA_SLICE_IMPORT_PROVIDER_AUTH:-0}"
SLICE_CODEX_PORT="${ARROBA_SLICE_CODEX_PORT:-43252}"
SLICE_OPENCODE_PORT="${ARROBA_SLICE_OPENCODE_PORT:-43140}"
SLICE_KERNEL_PORT="${ARROBA_SLICE_KERNEL_PORT:-43119}"
SLICE_MCP_PORT="${ARROBA_SLICE_MCP_PORT:-43120}"
SLICE_RELAY_PORT="${ARROBA_SLICE_RELAY_PORT:-43130}"
SLICE_NOVNC_PORT="${ARROBA_SLICE_NOVNC_PORT:-6080}"
SLICE_RELAY_URL="${ARROBA_SLICE_RELAY_URL:-}"
SLICE_RELAY_TOKEN="${ARROBA_SLICE_RELAY_TOKEN:-slice-local}"
SLICE_DAEMON_ALIAS="${ARROBA_SLICE_DAEMON_ALIAS:-slice:linux}"
SLICE_MACHINE_ID="${ARROBA_SLICE_MACHINE_ID:-slice:linux}"
SLICE_MACHINE_ALIAS="${ARROBA_SLICE_MACHINE_ALIAS:-linux}"
SLICE_CODEX_AUTH="${ARROBA_SLICE_CODEX_AUTH:-$HOME/.codex/auth.json}"
SLICE_OPENCODE_AUTH="${ARROBA_SLICE_OPENCODE_AUTH:-$HOME/.local/share/opencode/auth.json}"
SLICE_OPENCODE_PROVIDER="${ARROBA_SLICE_OPENCODE_PROVIDER:-openai}"
SLICE_OPENCODE_LOGIN_METHOD="${ARROBA_SLICE_OPENCODE_LOGIN_METHOD:-ChatGPT Pro/Plus (headless)}"

log() {
  printf '[slice-spike] %s\n' "$*" >&2
}

fail() {
  printf '[slice-spike] error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<EOF
Usage: $(basename "$0") [provision|status|stop|import-provider-auth|start-desktop|validate-screen|start-runtime|start-providers|shell]
       $(basename "$0") [login-codex|logout-codex|login-opencode|logout-opencode]

This Docker path is a provider/runtime validation fallback for Mac hosts when
the Lume Ubuntu prebuilt image is unavailable.
EOF
}

require_docker() {
  command -v docker >/dev/null || fail "docker is required"
  docker info >/dev/null || fail "docker is not running"
}

build_image() {
  log "building $SLICE_IMAGE"
  docker build \
    -f "$REPO_ROOT/experiments/slice-spike/docker/Dockerfile" \
    -t "$SLICE_IMAGE" \
    "$REPO_ROOT"
}

ensure_container() {
  if [[ "$SLICE_RECREATE" == "1" ]] && docker ps -a --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "recreating container $SLICE_NAME"
    docker rm -f "$SLICE_NAME" >/dev/null
  fi

  if docker ps -a --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "container $SLICE_NAME already exists"
  else
    log "creating container $SLICE_NAME"
    docker volume create "$SLICE_HOME_VOLUME" >/dev/null
    docker create \
      --name "$SLICE_NAME" \
      --security-opt seccomp=unconfined \
      -p "127.0.0.1:$SLICE_CODEX_PORT:$SLICE_CODEX_PORT" \
      -p "127.0.0.1:$SLICE_OPENCODE_PORT:$SLICE_OPENCODE_PORT" \
      -p "127.0.0.1:$SLICE_KERNEL_PORT:$SLICE_KERNEL_PORT" \
      -p "127.0.0.1:$SLICE_RELAY_PORT:$SLICE_RELAY_PORT" \
      -p "127.0.0.1:$SLICE_NOVNC_PORT:$SLICE_NOVNC_PORT" \
      -v "$SLICE_HOME_VOLUME:/home/slice" \
      -v "$SLICE_WORKSPACE:/workspace" \
      "$SLICE_IMAGE" >/dev/null
  fi

  if ! docker ps --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "starting container $SLICE_NAME"
    docker start "$SLICE_NAME" >/dev/null
  fi

  docker exec -u root "$SLICE_NAME" bash -lc "mkdir -p /home/slice/.local/share /home/slice/.config /home/slice/.cache && chown -R slice:slice /home/slice"
}

exec_slice() {
  docker exec \
    -e ARROBA_SLICE_CODEX_PORT="$SLICE_CODEX_PORT" \
    -e ARROBA_SLICE_OPENCODE_PORT="$SLICE_OPENCODE_PORT" \
    -e ARROBA_SLICE_KERNEL_PORT="$SLICE_KERNEL_PORT" \
    -e ARROBA_SLICE_MCP_PORT="$SLICE_MCP_PORT" \
    -e ARROBA_SLICE_RELAY_PORT="$SLICE_RELAY_PORT" \
    -e ARROBA_SLICE_NOVNC_PORT="$SLICE_NOVNC_PORT" \
    -e ARROBA_SLICE_RELAY_URL="$SLICE_RELAY_URL" \
    -e ARROBA_SLICE_RELAY_TOKEN="$SLICE_RELAY_TOKEN" \
    -e ARROBA_SLICE_DAEMON_ALIAS="$SLICE_DAEMON_ALIAS" \
    -e ARROBA_SLICE_MACHINE_ID="$SLICE_MACHINE_ID" \
    -e ARROBA_SLICE_MACHINE_ALIAS="$SLICE_MACHINE_ALIAS" \
    -u slice \
    "$SLICE_NAME" \
    "$@"
}

copy_provider_auth_file() {
  local source_path="$1"
  local target_path="$2"
  local label="$3"

  if [[ ! -f "$source_path" ]]; then
    log "$label auth not found at $source_path; skipping"
    return 0
  fi

  local target_dir
  target_dir="$(dirname "$target_path")"
  local backup_path="${target_path}.before-slice-auth-$(date +%Y%m%d%H%M%S)"
  docker exec -u slice "$SLICE_NAME" bash -lc "
    set -euo pipefail
    mkdir -p '$target_dir'
    if [[ -f '$target_path' ]]; then
      cp '$target_path' '$backup_path'
    fi
  "
  docker cp "$source_path" "$SLICE_NAME:$target_path"
  docker exec -u root "$SLICE_NAME" bash -lc "
    set -euo pipefail
    chown slice:slice '$target_path'
    chmod 600 '$target_path'
  "
  log "imported $label auth into $target_path"
}

import_provider_auth() {
  ensure_container
  copy_provider_auth_file "$SLICE_CODEX_AUTH" "/home/slice/.codex/auth.json" "Codex"
  copy_provider_auth_file "$SLICE_OPENCODE_AUTH" "/home/slice/.local/share/opencode/auth.json" "OpenCode"
}

print_provider_auth_status() {
  exec_slice bash -lc "echo '--- provider auth'; codex login status || true; opencode providers list || true"
}

print_status() {
  log "container: $SLICE_NAME"
  exec_slice bash -lc "set -e; echo '--- versions'; node --version; npm --version; codex --version || true; opencode --version || true; chromium --version || true; tesseract --version | head -n 1 || true; echo '--- browser smoke'; chromium --headless=new --disable-gpu --dump-dom 'data:text/html,slice-browser-ok' >/tmp/chromium-smoke.out 2>/tmp/chromium-smoke.err || { cat /tmp/chromium-smoke.err; exit 1; }; grep -q 'slice-browser-ok' /tmp/chromium-smoke.out && echo chromium=sandboxed-headless-ok; echo '--- desktop'; /opt/arroba-slice/slice-screen.sh status || true; echo '--- binaries'; ls -l /opt/arroba-slice/bin; echo '--- processes'; pgrep -af 'arroba-kernel|arroba-relay|codex app-server|opencode serve' || true; echo '--- logs'; ls -1 /opt/arroba-slice/logs || true"
  print_provider_auth_status
}

stop_container() {
  if ! docker ps -a --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "container $SLICE_NAME does not exist"
    return 0
  fi
  if docker ps --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "stopping slice processes in $SLICE_NAME"
    docker exec -u slice "$SLICE_NAME" bash -lc "
      screen -S arroba-slice-relay -X quit >/dev/null 2>&1 || true
      screen -S arroba-slice-kernel -X quit >/dev/null 2>&1 || true
      /opt/arroba-slice/slice-screen.sh stop >/dev/null 2>&1 || true
      pkill -f 'codex app-server' >/dev/null 2>&1 || true
      pkill -f 'opencode serve' >/dev/null 2>&1 || true
    " || true
    docker stop "$SLICE_NAME" >/dev/null
  else
    log "container $SLICE_NAME is already stopped"
  fi
}

main() {
  local action="${1:-provision}"
  case "$action" in
    -h|--help|help)
      usage
      ;;
    provision)
      require_docker
      build_image
      ensure_container
      if [[ "$SLICE_IMPORT_PROVIDER_AUTH" == "1" ]]; then
        import_provider_auth
      fi
      if [[ "$SLICE_START_DESKTOP" == "1" ]]; then
        exec_slice /opt/arroba-slice/slice-screen.sh start || true
      fi
      if [[ "$SLICE_START_RUNTIME" == "1" ]]; then
        exec_slice /opt/arroba-slice/start-runtime.sh || true
      fi
      if [[ "$SLICE_START_PROVIDER_SERVERS" == "1" ]]; then
        exec_slice /opt/arroba-slice/start-providers.sh || true
      fi
      print_status
      ;;
    status)
      require_docker
      print_status
      ;;
    stop)
      require_docker
      stop_container
      ;;
    import-provider-auth)
      require_docker
      import_provider_auth
      print_provider_auth_status
      ;;
    login-codex)
      require_docker
      ensure_container
      docker exec -it -u slice "$SLICE_NAME" codex login --device-auth
      ;;
    logout-codex)
      require_docker
      ensure_container
      exec_slice codex logout
      ;;
    login-opencode)
      require_docker
      ensure_container
      docker exec -it -u slice "$SLICE_NAME" opencode providers login \
        -p "$SLICE_OPENCODE_PROVIDER" \
        -m "$SLICE_OPENCODE_LOGIN_METHOD"
      ;;
    logout-opencode)
      require_docker
      ensure_container
      docker exec -it -u slice "$SLICE_NAME" opencode providers logout
      ;;
    start-desktop)
      require_docker
      ensure_container
      exec_slice /opt/arroba-slice/slice-screen.sh start
      ;;
    validate-screen)
      require_docker
      ensure_container
      exec_slice /opt/arroba-slice/validate-screen.sh prepare
      exec_slice /opt/arroba-slice/validate-screen.sh interact
      ;;
    start-runtime)
      require_docker
      ensure_container
      exec_slice /opt/arroba-slice/start-runtime.sh
      ;;
    start-providers)
      require_docker
      ensure_container
      exec_slice /opt/arroba-slice/start-providers.sh
      ;;
    shell)
      require_docker
      ensure_container
      docker exec -it -u slice "$SLICE_NAME" bash
      ;;
    *)
      usage
      fail "unknown action: $action"
      ;;
  esac
}

main "$@"
