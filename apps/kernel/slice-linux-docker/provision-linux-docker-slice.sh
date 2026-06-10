#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

SLICE_NAME="${ARROBA_SLICE_NAME:-arroba-slice-linux}"
SLICE_IMAGE="${ARROBA_SLICE_DOCKER_IMAGE:-arroba-slice-linux:0.1.0}"
SLICE_BASE_IMAGE="${ARROBA_SLICE_BASE_IMAGE:-arroba-slice-linux:0.1.0}"
SLICE_BUILD_IMAGE="${ARROBA_SLICE_BUILD_IMAGE:-auto}"
SLICE_EXTENSION_DOCKERFILE="${ARROBA_SLICE_EXTENSION_DOCKERFILE:-}"
SLICE_DOCKER_MEMORY="${ARROBA_SLICE_DOCKER_MEMORY:-}"
SLICE_DOCKER_CPUS="${ARROBA_SLICE_DOCKER_CPUS:-}"
SLICE_HOME_VOLUME="${ARROBA_SLICE_HOME_VOLUME:-${SLICE_NAME}-home}"
SLICE_SAVED_HOME_ARCHIVE="${ARROBA_SLICE_SAVED_HOME_ARCHIVE:-}"
SLICE_WORKSPACE="${ARROBA_SLICE_WORKSPACE:-$REPO_ROOT}"
SLICE_WORKSPACE_MOUNT_MODE="${ARROBA_SLICE_WORKSPACE_MOUNT_MODE:-rw}"
SLICE_ALLOW_UNCONFINED_SECCOMP="${ARROBA_SLICE_ALLOW_UNCONFINED_SECCOMP:-0}"
SLICE_RECREATE="${ARROBA_SLICE_RECREATE:-0}"
SLICE_START_DESKTOP="${ARROBA_SLICE_START_DESKTOP:-1}"
SLICE_START_PROVIDER_SERVERS="${ARROBA_SLICE_START_PROVIDER_SERVERS:-1}"
SLICE_START_RUNTIME="${ARROBA_SLICE_START_RUNTIME:-0}"
SLICE_IMPORT_PROVIDER_AUTH="${ARROBA_SLICE_IMPORT_PROVIDER_AUTH:-0}"
SLICE_MIN_FREE_MB="${ARROBA_SLICE_MIN_FREE_MB:-256}"
SLICE_CODEX_PORT="${ARROBA_SLICE_CODEX_PORT:-43252}"
SLICE_OPENCODE_PORT="${ARROBA_SLICE_OPENCODE_PORT:-43140}"
SLICE_CODEX_PORT_RANGE="${ARROBA_SLICE_CODEX_PORT_RANGE:-43260-43279}"
SLICE_OPENCODE_PORT_RANGE="${ARROBA_SLICE_OPENCODE_PORT_RANGE:-43150-43169}"
SLICE_PROVIDER_BIND_HOST="${ARROBA_SLICE_PROVIDER_BIND_HOST:-127.0.0.1}"
SLICE_KERNEL_PORT="${ARROBA_SLICE_KERNEL_PORT:-43119}"
SLICE_MCP_PORT="${ARROBA_SLICE_MCP_PORT:-43120}"
SLICE_RELAY_PORT="${ARROBA_SLICE_RELAY_PORT:-43130}"
SLICE_NOVNC_PORT="${ARROBA_SLICE_NOVNC_PORT:-6080}"
SLICE_RELAY_URL="${ARROBA_SLICE_RELAY_URL:-}"
SLICE_RELAY_TOKEN="${ARROBA_SLICE_RELAY_TOKEN:-slice-local}"
SLICE_CLOUD_RELAY_CONFIG_JSON="${ARROBA_SLICE_CLOUD_RELAY_CONFIG_JSON:-}"
SLICE_CLOUD_RELAY_CONFIG_HOST_PATH="${ARROBA_SLICE_CLOUD_RELAY_CONFIG_HOST_PATH:-}"
SLICE_DAEMON_ALIAS="${ARROBA_SLICE_DAEMON_ALIAS:-slice:linux}"
SLICE_MACHINE_ID="${ARROBA_SLICE_MACHINE_ID:-slice:linux}"
SLICE_MACHINE_ALIAS="${ARROBA_SLICE_MACHINE_ALIAS:-linux}"
SLICE_CODEX_AUTH="${ARROBA_SLICE_CODEX_AUTH:-$HOME/.codex/auth.json}"
SLICE_OPENCODE_AUTH="${ARROBA_SLICE_OPENCODE_AUTH:-$HOME/.local/share/opencode/auth.json}"
SLICE_CLAUDE_JSON="${ARROBA_SLICE_CLAUDE_JSON:-$HOME/.claude.json}"
SLICE_CLAUDE_SETTINGS="${ARROBA_SLICE_CLAUDE_SETTINGS:-$HOME/.claude/settings.json}"
SLICE_CLAUDE_STATS="${ARROBA_SLICE_CLAUDE_STATS:-$HOME/.claude/stats-cache.json}"
SLICE_CLAUDE_CREDENTIALS="${ARROBA_SLICE_CLAUDE_CREDENTIALS:-$HOME/.claude/.credentials.json}"
SLICE_CLAUDE_KEYCHAIN_SERVICE="${ARROBA_SLICE_CLAUDE_KEYCHAIN_SERVICE:-Claude Code-credentials}"
SLICE_OPENCODE_PROVIDER="${ARROBA_SLICE_OPENCODE_PROVIDER:-openai}"
SLICE_OPENCODE_LOGIN_METHOD="${ARROBA_SLICE_OPENCODE_LOGIN_METHOD:-ChatGPT Pro/Plus (headless)}"
SLICE_LOGIN_PROVIDER="${ARROBA_SLICE_LOGIN_PROVIDER:-codex}"
SLICE_AUTH_PROVIDER="${ARROBA_SLICE_AUTH_PROVIDER:-all}"

log() {
  printf '[slice-linux] %s\n' "$*" >&2
}

fail() {
  printf '[slice-linux] error: %s\n' "$*" >&2
  exit 1
}

run_with_timeout() {
  local seconds="$1"
  shift
  local command_display="$*"
  local timeout_marker="${TMPDIR:-/tmp}/arroba-slice-timeout.$$.$RANDOM"
  rm -f "$timeout_marker"
  "$@" &
  local child=$!
  (
    sleep "$seconds"
    if kill -0 "$child" >/dev/null 2>&1; then
      : >"$timeout_marker"
      kill "$child" >/dev/null 2>&1 || true
      sleep 2
      kill -9 "$child" >/dev/null 2>&1 || true
    fi
  ) &
  local watchdog=$!
  local status=0
  wait "$child" || status=$?
  kill "$watchdog" >/dev/null 2>&1 || true
  wait "$watchdog" 2>/dev/null || true
  if [[ -f "$timeout_marker" ]]; then
    rm -f "$timeout_marker"
    log "timed out after ${seconds}s: ${command_display}"
    return 124
  fi
  rm -f "$timeout_marker"
  return "$status"
}

run_with_file_stdin_timeout() {
  local seconds="$1"
  local input_file="$2"
  shift 2
  local command_display="$* < $input_file"
  local timeout_marker="${TMPDIR:-/tmp}/arroba-slice-timeout.$$.$RANDOM"
  rm -f "$timeout_marker"
  "$@" <"$input_file" &
  local child=$!
  (
    sleep "$seconds"
    if kill -0 "$child" >/dev/null 2>&1; then
      : >"$timeout_marker"
      kill "$child" >/dev/null 2>&1 || true
      sleep 2
      kill -9 "$child" >/dev/null 2>&1 || true
    fi
  ) &
  local watchdog=$!
  local status=0
  wait "$child" || status=$?
  kill "$watchdog" >/dev/null 2>&1 || true
  wait "$watchdog" 2>/dev/null || true
  if [[ -f "$timeout_marker" ]]; then
    rm -f "$timeout_marker"
    log "timed out after ${seconds}s: ${command_display}"
    return 124
  fi
  rm -f "$timeout_marker"
  return "$status"
}

usage() {
  cat <<EOF
Usage: $(basename "$0") [provision|status|stop|destroy|import-provider-auth|remove-provider-auth|start-provider-login|start-desktop|validate-screen|start-runtime|start-providers|shell]
       $(basename "$0") [login-codex|logout-codex|login-opencode|logout-opencode]

This Docker path is a provider/runtime validation fallback for Mac hosts when
the Lume Ubuntu prebuilt image is unavailable.
EOF
}

require_docker() {
  command -v docker >/dev/null || fail "docker is required"
  run_with_timeout 20 docker info >/dev/null || fail "docker is not running"
}

container_exists() {
  run_with_timeout 20 docker container inspect "$SLICE_NAME" >/dev/null 2>&1
}

container_running() {
  local state
  state="$(run_with_timeout 20 docker inspect -f '{{.State.Running}}' "$SLICE_NAME" 2>/dev/null)" || return 1
  [[ "$state" == "true" ]]
}

restore_saved_home_volume() {
  [[ -n "$SLICE_SAVED_HOME_ARCHIVE" ]] || return 0
  [[ -f "$SLICE_SAVED_HOME_ARCHIVE" ]] || fail "saved slice home archive not found: $SLICE_SAVED_HOME_ARCHIVE"
  local helper
  helper="${SLICE_NAME}-home-restore-$$"
  log "restoring saved home archive $SLICE_SAVED_HOME_ARCHIVE into volume $SLICE_HOME_VOLUME"
  run_with_timeout 30 docker rm -f "$helper" >/dev/null 2>&1 || true
  run_with_timeout 60 docker create --name "$helper" --user root \
    -v "$SLICE_HOME_VOLUME:/home-dst" \
    "$SLICE_IMAGE" \
    sleep infinity >/dev/null
  run_with_timeout 60 docker start "$helper" >/dev/null
  run_with_timeout 120 docker cp "$SLICE_SAVED_HOME_ARCHIVE" "$helper:/tmp/home.tar.zst"
  run_with_timeout 120 docker exec -u root "$helper" \
    bash -lc "set -euo pipefail; find /home-dst -mindepth 1 -maxdepth 1 -exec rm -rf {} +; cd /home-dst; tar --zstd -xf /tmp/home.tar.zst; chown -R slice:slice /home-dst"
  run_with_timeout 30 docker rm -f "$helper" >/dev/null 2>&1 || true
}

machine_id_hex() {
  printf '%s' "$SLICE_MACHINE_ID" | sha256sum | awk '{ print substr($1, 1, 32) }'
}

configure_stable_machine_identity() {
  local machine_id
  machine_id="$(machine_id_hex)"
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" bash -lc "
    set -euo pipefail
    printf '%s\n' '$machine_id' > /etc/machine-id
    mkdir -p /var/lib/dbus
    printf '%s\n' '$machine_id' > /var/lib/dbus/machine-id
    chmod 0444 /etc/machine-id /var/lib/dbus/machine-id
  " || log "stable machine-id refresh unavailable; continuing"
}

configure_chromium_browser_policy() {
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" bash -lc "
    set -euo pipefail
    for dir in /etc/chromium/policies/managed /etc/chromium-browser/policies/managed; do
      mkdir -p \"\$dir\"
      cat > \"\$dir/arroba-slice.json\" <<'JSON'
{\"BrowserSignin\":0}
JSON
      chmod 0644 \"\$dir/arroba-slice.json\"
    done
  " || log "Chromium browser policy refresh unavailable; continuing"
}

refresh_slice_support_files() {
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/start-runtime.sh" "$SLICE_NAME:/opt/arroba-slice/start-runtime.sh" \
    || log "runtime script overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/slice-screen.sh" "$SLICE_NAME:/opt/arroba-slice/slice-screen.sh" \
    || log "screen script overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-cdp.mjs" "$SLICE_NAME:/opt/arroba-slice/browser-cdp.mjs" \
    || log "browser CDP helper overlay refresh unavailable; continuing"
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" chmod +x /opt/arroba-slice/start-runtime.sh /opt/arroba-slice/slice-screen.sh /opt/arroba-slice/browser-cdp.mjs \
    || log "script permission refresh unavailable; continuing"
}

wait_for_container_running() {
  local attempts="${1:-6}"
  local delay_seconds="${2:-5}"
  local attempt
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    if container_running; then
      return 0
    fi
    sleep "$delay_seconds"
  done
  return 1
}

available_mb_for_path() {
  local path="$1"
  run_with_timeout 20 docker exec -u slice "$SLICE_NAME" df -Pm "$path" 2>/dev/null | awk 'NR == 2 { print $4 }'
}

require_slice_free_space() {
  local phase="$1"
  shift
  [[ "$SLICE_MIN_FREE_MB" =~ ^[0-9]+$ ]] || fail "ARROBA_SLICE_MIN_FREE_MB must be a non-negative integer"
  local paths=("$@")
  local path
  for path in "${paths[@]}"; do
    local available_mb=""
    available_mb="$(available_mb_for_path "$path" || true)"
    if [[ ! "$available_mb" =~ ^[0-9]+$ ]]; then
      log "slice storage preflight unavailable for $path during $phase; continuing"
      continue
    fi
    if (( available_mb < SLICE_MIN_FREE_MB )); then
      log "slice storage preflight failed for $phase: $path has ${available_mb}MiB free, needs ${SLICE_MIN_FREE_MB}MiB"
      run_with_timeout 10 docker exec -u slice "$SLICE_NAME" df -h "${paths[@]}" >&2 || true
      fail "slice $phase needs more free space in the Docker/Colima slice filesystem. Free Docker disk or delete unused slice containers/volumes, then retry."
    fi
    log "slice storage preflight ok for $phase: $path has ${available_mb}MiB free"
  done
}

build_image() {
  case "$SLICE_BUILD_IMAGE" in
    auto|always|never) ;;
    *) fail "ARROBA_SLICE_BUILD_IMAGE must be auto, always, or never" ;;
  esac

  if [[ "$SLICE_BUILD_IMAGE" == "never" ]]; then
    if docker image inspect "$SLICE_IMAGE" >/dev/null 2>&1; then
      log "using existing $SLICE_IMAGE"
      return 0
    fi
    fail "Docker image $SLICE_IMAGE does not exist and build policy is never"
  fi

  if [[ "$SLICE_BUILD_IMAGE" == "auto" ]] && docker image inspect "$SLICE_IMAGE" >/dev/null 2>&1; then
    log "using cached $SLICE_IMAGE"
    return 0
  fi

  log "building $SLICE_IMAGE"
  if [[ -n "$SLICE_EXTENSION_DOCKERFILE" ]]; then
    [[ -f "$SLICE_EXTENSION_DOCKERFILE" ]] || fail "extension Dockerfile not found: $SLICE_EXTENSION_DOCKERFILE"
    docker build \
      --build-arg "ARROBA_SLICE_BASE_IMAGE=$SLICE_BASE_IMAGE" \
      -f "$SLICE_EXTENSION_DOCKERFILE" \
      -t "$SLICE_IMAGE" \
      "$(dirname "$SLICE_EXTENSION_DOCKERFILE")"
    return 0
  fi
  docker build \
    -f "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/Dockerfile" \
    -t "$SLICE_IMAGE" \
    "$REPO_ROOT"
}

ensure_container() {
  local created_container=0
  if [[ "$SLICE_RECREATE" == "1" ]] && container_exists; then
    log "recreating container $SLICE_NAME"
    run_with_timeout 60 docker rm -f "$SLICE_NAME" >/dev/null
  fi
  case "$SLICE_WORKSPACE_MOUNT_MODE" in
    rw|ro) ;;
    *) fail "ARROBA_SLICE_WORKSPACE_MOUNT_MODE must be rw or ro" ;;
  esac
  case "$SLICE_ALLOW_UNCONFINED_SECCOMP" in
    0|1) ;;
    *) fail "ARROBA_SLICE_ALLOW_UNCONFINED_SECCOMP must be 0 or 1" ;;
  esac

  if container_exists; then
    log "container $SLICE_NAME already exists"
  else
    log "creating container $SLICE_NAME"
    run_with_timeout 30 docker volume create "$SLICE_HOME_VOLUME" >/dev/null
    restore_saved_home_volume
    local docker_create_args=(
      --name "$SLICE_NAME"
      --ulimit core=0:0
      -p "127.0.0.1:$SLICE_CODEX_PORT:$SLICE_CODEX_PORT"
      -p "127.0.0.1:$SLICE_OPENCODE_PORT:$SLICE_OPENCODE_PORT"
      -p "127.0.0.1:$SLICE_CODEX_PORT_RANGE:$SLICE_CODEX_PORT_RANGE"
      -p "127.0.0.1:$SLICE_OPENCODE_PORT_RANGE:$SLICE_OPENCODE_PORT_RANGE"
      -p "127.0.0.1:$SLICE_KERNEL_PORT:$SLICE_KERNEL_PORT"
      -p "127.0.0.1:$SLICE_RELAY_PORT:$SLICE_RELAY_PORT"
      -p "127.0.0.1:$SLICE_NOVNC_PORT:$SLICE_NOVNC_PORT"
      -v "$SLICE_HOME_VOLUME:/home/slice"
      -v "$SLICE_WORKSPACE:/workspace:$SLICE_WORKSPACE_MOUNT_MODE"
      --add-host "host.docker.internal:host-gateway"
    )
    if [[ "$SLICE_ALLOW_UNCONFINED_SECCOMP" == "1" ]]; then
      docker_create_args+=(--security-opt seccomp=unconfined)
    fi
    if [[ "$SLICE_WORKSPACE" != "/workspace" ]]; then
      docker_create_args+=(-v "$SLICE_WORKSPACE:$SLICE_WORKSPACE:$SLICE_WORKSPACE_MOUNT_MODE")
    fi
    if [[ -n "$SLICE_DOCKER_MEMORY" ]]; then
      docker_create_args+=(--memory "$SLICE_DOCKER_MEMORY")
    fi
    if [[ -n "$SLICE_DOCKER_CPUS" ]]; then
      docker_create_args+=(--cpus "$SLICE_DOCKER_CPUS")
    fi
    local create_status=0
    if run_with_timeout 60 docker create "${docker_create_args[@]}" "$SLICE_IMAGE" >/dev/null; then
      create_status=0
    else
      create_status=$?
    fi
    if [[ "$create_status" -ne 0 ]]; then
      if container_exists; then
        log "docker create returned $create_status but container exists; continuing"
      else
        return "$create_status"
      fi
    fi
    created_container=1
  fi

  if ! container_running; then
    log "starting container $SLICE_NAME"
    local start_status=0
    if run_with_timeout 60 docker start "$SLICE_NAME" >/dev/null; then
      start_status=0
    else
      start_status=$?
    fi
    if [[ "$start_status" -ne 0 ]]; then
      if wait_for_container_running 24 5; then
        log "docker start returned $start_status but container is running; continuing"
      else
        log "docker start returned $start_status and container did not report running yet; continuing to verify with setup commands"
      fi
    fi
  fi

  if [[ "$created_container" == "1" ]]; then
    run_with_timeout 30 docker exec -u root "$SLICE_NAME" bash -lc "mkdir -p /home/slice/.local/share /home/slice/.config /home/slice/.cache && chown -R slice:slice /home/slice" \
      || log "home directory ownership refresh unavailable; continuing"
  fi
  configure_stable_machine_identity
  configure_chromium_browser_policy
  refresh_slice_support_files
}

exec_slice_with_timeout() {
  local seconds="$1"
  shift
  local relay_env_args=()
  if [[ -n "$SLICE_CLOUD_RELAY_CONFIG_HOST_PATH" || -n "$SLICE_CLOUD_RELAY_CONFIG_JSON" ]]; then
    local cloud_relay_config_path="/tmp/arroba-slice-state/cloud-relay-config.json"
    if [[ -n "$SLICE_CLOUD_RELAY_CONFIG_HOST_PATH" && -f "$SLICE_CLOUD_RELAY_CONFIG_HOST_PATH" ]]; then
      run_with_timeout 30 docker exec -u root "$SLICE_NAME" mkdir -p /tmp/arroba-slice-state
      run_with_timeout 30 docker cp "$SLICE_CLOUD_RELAY_CONFIG_HOST_PATH" "$SLICE_NAME:$cloud_relay_config_path"
      run_with_timeout 30 docker exec -u root "$SLICE_NAME" chown slice:slice "$cloud_relay_config_path"
      run_with_timeout 30 docker exec -u root "$SLICE_NAME" chmod 600 "$cloud_relay_config_path"
    else
      run_with_timeout 30 docker exec -i -u slice "$SLICE_NAME" bash -lc "set -euo pipefail; umask 077; mkdir -p /tmp/arroba-slice-state; cat > '$cloud_relay_config_path'" <<<"$SLICE_CLOUD_RELAY_CONFIG_JSON"
    fi
    relay_env_args+=(-e ARROBA_SLICE_CLOUD_RELAY_CONFIG_PATH="$cloud_relay_config_path")
  fi
  relay_env_args+=(-e ARROBA_SLICE_RELAY_TOKEN="$SLICE_RELAY_TOKEN")
  if [[ -n "$SLICE_RELAY_URL" ]]; then
    relay_env_args+=(-e ARROBA_SLICE_RELAY_URL="$SLICE_RELAY_URL")
  fi
  run_with_timeout "$seconds" docker exec \
    -e ARROBA_SLICE_CODEX_PORT="$SLICE_CODEX_PORT" \
    -e ARROBA_SLICE_OPENCODE_PORT="$SLICE_OPENCODE_PORT" \
    -e ARROBA_SLICE_CODEX_PORT_RANGE="$SLICE_CODEX_PORT_RANGE" \
    -e ARROBA_SLICE_OPENCODE_PORT_RANGE="$SLICE_OPENCODE_PORT_RANGE" \
    -e ARROBA_SLICE_PROVIDER_BIND_HOST="$SLICE_PROVIDER_BIND_HOST" \
    -e ARROBA_SLICE_KERNEL_PORT="$SLICE_KERNEL_PORT" \
    -e ARROBA_SLICE_MCP_PORT="$SLICE_MCP_PORT" \
    -e ARROBA_SLICE_RELAY_PORT="$SLICE_RELAY_PORT" \
    -e ARROBA_SLICE_NOVNC_PORT="$SLICE_NOVNC_PORT" \
    "${relay_env_args[@]}" \
    -e ARROBA_SLICE_DAEMON_ALIAS="$SLICE_DAEMON_ALIAS" \
    -e ARROBA_SLICE_MACHINE_ID="$SLICE_MACHINE_ID" \
    -e ARROBA_SLICE_MACHINE_ALIAS="$SLICE_MACHINE_ALIAS" \
    -e ARROBA_SLICE_SCREEN_GEOMETRY="${ARROBA_SLICE_SCREEN_GEOMETRY:-1280x800x24}" \
    -u slice \
    "$SLICE_NAME" \
    "$@"
}

exec_slice() {
  exec_slice_with_timeout 90 "$@"
}

slice_screen_diagnostics() {
  log "slice screen diagnostics"
  run_with_timeout 30 docker exec -u slice "$SLICE_NAME" bash -lc "
    set +e
    /opt/arroba-slice/slice-screen.sh status
    echo '--- processes'
    pgrep -af 'Xvfb|openbox|x11vnc|websockify|chromium' || true
    echo '--- logs'
    for log_file in /opt/arroba-slice/logs/xvfb.log /opt/arroba-slice/logs/openbox.log /opt/arroba-slice/logs/x11vnc.log /opt/arroba-slice/logs/novnc.log /opt/arroba-slice/logs/chromium-gui.log; do
      echo \"==== \${log_file}\"
      tail -n 40 \"\${log_file}\" 2>/dev/null || true
    done
  " >&2 || log "slice screen diagnostics unavailable"
}

run_required_phase() {
  local label="$1"
  shift
  log "starting phase: $label"
  local status=0
  "$@" || status=$?
  if [[ "$status" -eq 0 ]]; then
    log "completed phase: $label"
    return 0
  fi
  log "phase failed: $label (status $status)"
  case "$label" in
    desktop)
      slice_screen_diagnostics
      ;;
  esac
  return "$status"
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
  run_with_file_stdin_timeout 90 "$source_path" docker exec -i -u slice "$SLICE_NAME" bash -lc "
    set -euo pipefail
    mkdir -p '$target_dir'
    if [[ -f '$target_path' ]]; then
      cp '$target_path' '$backup_path'
    fi
    umask 077
    cat > '$target_path'
    chmod 600 '$target_path'
  "
  log "imported $label auth into $target_path"
}

trust_claude_slice_workspace() {
  if ! run_with_timeout 30 docker exec -u slice "$SLICE_NAME" bash -lc "node <<'NODE'
const fs = require('fs')
const file = '/home/slice/.claude.json'
let data = {}
try {
  data = JSON.parse(fs.readFileSync(file, 'utf8'))
} catch {
  data = {}
}
const projects = data.projects && typeof data.projects === 'object' ? data.projects : {}
const template = Object.values(projects).find((value) =>
  value && typeof value === 'object' && Object.prototype.hasOwnProperty.call(value, 'hasTrustDialogAccepted')
) || {}
projects['/workspace'] = {
  ...template,
  allowedTools: Array.isArray(template.allowedTools) ? template.allowedTools : [],
  hasTrustDialogAccepted: true,
  projectOnboardingSeenCount: Math.max(Number(template.projectOnboardingSeenCount) || 0, 1),
}
data.projects = projects
fs.writeFileSync(file, JSON.stringify(data, null, 2))
fs.chmodSync(file, 0o600)
NODE"
  then
    log "Claude workspace trust update unavailable; continuing"
    return 0
  fi
  log "marked /workspace as trusted for Claude Code"
}

import_provider_auth() {
  ensure_container
  require_slice_free_space "provider-auth" /home/slice /tmp
  case "$SLICE_AUTH_PROVIDER" in
    all)
      import_codex_auth
      import_opencode_auth
      import_claude_auth
      ;;
    codex)
      import_codex_auth
      ;;
    opencode|opencode:*)
      import_opencode_auth
      ;;
    claude)
      import_claude_auth
      ;;
    *)
      fail "unsupported provider auth import: $SLICE_AUTH_PROVIDER"
      ;;
  esac
}

remove_provider_auth() {
  ensure_container
  case "$SLICE_AUTH_PROVIDER" in
    all)
      remove_codex_auth
      remove_opencode_auth
      remove_claude_auth
      ;;
    codex)
      remove_codex_auth
      ;;
    opencode|opencode:*)
      remove_opencode_auth
      ;;
    claude)
      remove_claude_auth
      ;;
    *)
      fail "unsupported provider auth removal: $SLICE_AUTH_PROVIDER"
      ;;
  esac
}

import_codex_auth() {
  copy_provider_auth_file "$SLICE_CODEX_AUTH" "/home/slice/.codex/auth.json" "Codex"
}

remove_codex_auth() {
  exec_slice bash -lc "rm -f /home/slice/.codex/auth.json"
  log "removed Codex auth from slice"
}

import_opencode_auth() {
  copy_provider_auth_file "$SLICE_OPENCODE_AUTH" "/home/slice/.local/share/opencode/auth.json" "OpenCode"
}

remove_opencode_auth() {
  exec_slice bash -lc "rm -f /home/slice/.local/share/opencode/auth.json"
  log "removed OpenCode auth from slice"
}

import_claude_auth() {
  copy_provider_auth_file "$SLICE_CLAUDE_JSON" "/home/slice/.claude.json" "Claude metadata"
  copy_provider_auth_file "$SLICE_CLAUDE_SETTINGS" "/home/slice/.claude/settings.json" "Claude settings"
  copy_provider_auth_file "$SLICE_CLAUDE_STATS" "/home/slice/.claude/stats-cache.json" "Claude stats"
  if [[ -f "$SLICE_CLAUDE_CREDENTIALS" ]]; then
    copy_provider_auth_file "$SLICE_CLAUDE_CREDENTIALS" "/home/slice/.claude/.credentials.json" "Claude credentials"
  elif command -v security >/dev/null 2>&1; then
    local credentials_tmp
    credentials_tmp="$(mktemp "${TMPDIR:-/tmp}/arroba-claude-credentials.XXXXXX")"
    if security find-generic-password -s "$SLICE_CLAUDE_KEYCHAIN_SERVICE" -w >"$credentials_tmp" 2>/dev/null; then
      copy_provider_auth_file "$credentials_tmp" "/home/slice/.claude/.credentials.json" "Claude Keychain credentials"
    else
      log "Claude credentials not found in Keychain service $SLICE_CLAUDE_KEYCHAIN_SERVICE; skipping"
    fi
    rm -f "$credentials_tmp"
  else
    log "Claude credentials not found at $SLICE_CLAUDE_CREDENTIALS; skipping"
  fi
  trust_claude_slice_workspace
}

remove_claude_auth() {
  exec_slice bash -lc "rm -f /home/slice/.claude.json /home/slice/.claude/settings.json /home/slice/.claude/stats-cache.json /home/slice/.claude/.credentials.json"
  log "removed Claude auth from slice"
}

print_provider_auth_status() {
  if ! exec_slice_with_timeout 30 bash -lc "
    set +e
    probe() {
      local label=\"\$1\"
      shift
      if command -v timeout >/dev/null 2>&1; then
        timeout 8s \"\$@\"
        local status=\$?
        if [[ \$status -eq 124 ]]; then
          printf '%s probe timed out\\n' \"\$label\" >&2
        fi
        return \$status
      fi
      \"\$@\"
      return \$?
    }
    echo '--- provider auth'
    probe codex codex login status || true
    probe opencode opencode providers list || true
    probe claude claude auth status --text || probe claude claude auth status || true
  "; then
    log "provider auth status diagnostics unavailable"
  fi
}

provider_login_command() {
  case "$SLICE_LOGIN_PROVIDER" in
    codex)
      printf '%s\n' "codex login --device-auth"
      ;;
    opencode|opencode:openai)
      printf '%s\n' "opencode providers login -p openai -m '$SLICE_OPENCODE_LOGIN_METHOD'"
      ;;
    claude|claude:claudeai)
      printf '%s\n' "claude auth login --claudeai"
      ;;
    *)
      fail "unsupported slice provider login: $SLICE_LOGIN_PROVIDER"
      ;;
  esac
}

start_provider_login() {
  ensure_container
  require_slice_free_space "provider-login" /home/slice /tmp
  local safe_provider
  safe_provider="$(printf '%s' "$SLICE_LOGIN_PROVIDER" | tr -c 'A-Za-z0-9_.-' '-')"
  local session_name="arroba-slice-login-${safe_provider}"
  local log_file="/opt/arroba-slice/logs/provider-login-${safe_provider}.log"
  local command_text
  command_text="$(provider_login_command)"
  log "starting $SLICE_LOGIN_PROVIDER login in $session_name"
  run_with_timeout 30 docker exec -u slice "$SLICE_NAME" bash -lc "
    set -euo pipefail
    mkdir -p /opt/arroba-slice/logs
    rm -f '$log_file'
    screen -S '$session_name' -X quit >/dev/null 2>&1 || true
    screen -dmS '$session_name' bash -lc \"set +e; $command_text 2>&1 | tee -a '$log_file'; printf '\\n[arroba] provider login exited with status %s\\n' \\\${PIPESTATUS[0]} | tee -a '$log_file'; exec bash\"
  " || log "provider login start command did not confirm; continuing with screen fallback"
  sleep 3
  local login_output=""
  login_output="$(run_with_timeout 30 docker exec -u slice "$SLICE_NAME" bash -lc "cat '$log_file' 2>/dev/null || true")" || true
  if [[ -n "$login_output" ]]; then
    printf '%s\n' "$login_output"
  else
    printf '[arroba] provider login started in screen session %s; open the slice screen or slice logs to continue\n' "$session_name"
  fi
}

print_status() {
  log "container: $SLICE_NAME"
  if ! exec_slice_with_timeout 30 bash -lc "
    set +e
    probe() {
      local label=\"\$1\"
      shift
      if command -v timeout >/dev/null 2>&1; then
        timeout 8s \"\$@\"
        local status=\$?
        if [[ \$status -eq 124 ]]; then
          printf '%s probe timed out\\n' \"\$label\" >&2
        fi
        return \$status
      fi
      \"\$@\"
      return \$?
    }
    echo '--- versions'
    probe node node --version || true
    probe npm npm --version || true
    probe codex codex --version || true
    probe opencode opencode --version || true
    probe claude claude --version || true
    probe chromium chromium --version || true
    probe tesseract tesseract --version | head -n 1 || true
    echo '--- browser smoke'
    if probe chromium-headless chromium --headless=new --no-sandbox --disable-gpu --dump-dom 'data:text/html,slice-browser-ok' >/tmp/chromium-smoke.out 2>/tmp/chromium-smoke.err; then
      grep -q 'slice-browser-ok' /tmp/chromium-smoke.out && echo chromium=headless-ok
    else
      cat /tmp/chromium-smoke.err 2>/dev/null || true
      echo chromium=headless-unavailable
    fi
    echo '--- desktop'
    probe slice-screen /opt/arroba-slice/slice-screen.sh status || true
    echo '--- binaries'
    probe binaries ls -l /opt/arroba-slice/bin || true
    echo '--- processes'
    probe processes pgrep -af 'arroba-kernel|arroba-relay|codex app-server|opencode serve' || true
    echo '--- logs'
    probe logs ls -1 /opt/arroba-slice/logs || true
  "; then
    log "status diagnostics unavailable"
  fi
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

destroy_container() {
  stop_container
  if docker ps -a --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "removing container $SLICE_NAME"
    docker rm "$SLICE_NAME" >/dev/null
  fi
  if docker volume inspect "$SLICE_HOME_VOLUME" >/dev/null 2>&1; then
    log "removing volume $SLICE_HOME_VOLUME"
    docker volume rm "$SLICE_HOME_VOLUME" >/dev/null
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
        require_slice_free_space "desktop" /home/slice /tmp
        run_required_phase desktop exec_slice_with_timeout 60 bash -lc "/opt/arroba-slice/slice-screen.sh start"
      fi
      if [[ "$SLICE_START_RUNTIME" == "1" ]]; then
        require_slice_free_space "runtime" /home/slice /tmp
        run_required_phase runtime exec_slice /opt/arroba-slice/start-runtime.sh
      fi
      if [[ "$SLICE_START_PROVIDER_SERVERS" == "1" ]]; then
        run_required_phase provider-servers exec_slice /opt/arroba-slice/start-providers.sh
      fi
      log "provision completed; use status or logs actions for diagnostics"
      ;;
    status)
      require_docker
      print_status
      ;;
    stop)
      require_docker
      stop_container
      ;;
    destroy)
      require_docker
      destroy_container
      ;;
    import-provider-auth)
      require_docker
      import_provider_auth
      log "provider auth import completed; account summaries are extracted by the home kernel"
      ;;
    remove-provider-auth)
      require_docker
      remove_provider_auth
      log "provider auth removal completed; account summaries are reconciled by the home kernel"
      ;;
    start-provider-login)
      require_docker
      start_provider_login
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
      require_slice_free_space "desktop" /home/slice /tmp
      run_required_phase desktop exec_slice_with_timeout 60 bash -lc "/opt/arroba-slice/slice-screen.sh start"
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
      require_slice_free_space "runtime" /home/slice /tmp
      exec_slice /opt/arroba-slice/start-runtime.sh
      ;;
    start-providers)
      require_docker
      ensure_container
      require_slice_free_space "provider-servers" /home/slice /tmp
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
