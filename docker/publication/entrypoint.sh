#!/usr/bin/env bash
set -euo pipefail
umask 077

export ARROBA_PUBLICATION_RUNTIME_STATE_DIR="${ARROBA_PUBLICATION_RUNTIME_STATE_DIR:-$ARROBA_DATA_DIR/publication-runtime}"
export ARROBA_WORKSPACE_DIR="${ARROBA_WORKSPACE_DIR:-/workspace}"
readonly ARROBA_ACTION_HOME=/home/arroba-action
readonly ARROBA_GATEWAY_HOME=/home/arroba-gateway
readonly ARROBA_KERNEL_TMPDIR="$HOME/.tmp"
readonly ARROBA_ACTION_TMPDIR="$ARROBA_ACTION_HOME/.tmp"
readonly ARROBA_GATEWAY_TMPDIR="$ARROBA_GATEWAY_HOME/.tmp"
readonly ARROBA_CAPABILITY_ROOT=/run/arroba-publication-capabilities
readonly ARROBA_CAPABILITY_STAGING_DIR="$ARROBA_CAPABILITY_ROOT/.staging"
readonly ARROBA_KERNEL_CAPABILITY_DIR="$ARROBA_CAPABILITY_ROOT/kernel"
readonly ARROBA_GATEWAY_CAPABILITY_DIR="$ARROBA_CAPABILITY_ROOT/gateway"
readonly ARROBA_KERNEL_AUTH_FILE="$ARROBA_KERNEL_CAPABILITY_DIR/kernel-local-auth"
readonly ARROBA_GATEWAY_AUTH_FILE="$ARROBA_GATEWAY_CAPABILITY_DIR/kernel-local-auth"
readonly ARROBA_GATEWAY_AUDIT_FILE="$ARROBA_GATEWAY_CAPABILITY_DIR/publication-audit-url"
readonly ARROBA_GATEWAY_RUNTIME_STATE_DIR="$ARROBA_GATEWAY_HOME/.local/share/arroba/publication-runtime"

mkdir -p \
  "$ARROBA_CONFIG_DIR" \
  "$ARROBA_DATA_DIR" \
  "$ARROBA_RUNTIME_DIR" \
  "$ARROBA_SESSION_HISTORY_DIR" \
  "$ARROBA_PUBLICATION_RUNTIME_STATE_DIR" \
  "$HOME/.cache" \
  "$ARROBA_KERNEL_TMPDIR" \
  "$ARROBA_WORKSPACE_DIR"

generate_kernel_local_auth_token() {
  node -e 'process.stdout.write(require("node:crypto").randomBytes(32).toString("hex"))'
}

prepare_capability_directories() {
  if [[ "$(id -u)" -ne 0 ]]; then
    echo "publication standalone mode must start as root to provision capabilities" >&2
    return 70
  fi
  local path
  for path in \
    "$ARROBA_CAPABILITY_ROOT" \
    "$ARROBA_CAPABILITY_STAGING_DIR" \
    "$ARROBA_KERNEL_CAPABILITY_DIR" \
    "$ARROBA_GATEWAY_CAPABILITY_DIR"; do
    if [[ -L "$path" ]]; then
      echo "publication capability directory must not be a symlink: $path" >&2
      return 70
    fi
  done
  mkdir -p \
    "$ARROBA_CAPABILITY_STAGING_DIR" \
    "$ARROBA_KERNEL_CAPABILITY_DIR" \
    "$ARROBA_GATEWAY_CAPABILITY_DIR"
  chown root:root "$ARROBA_CAPABILITY_ROOT" "$ARROBA_CAPABILITY_STAGING_DIR"
  chown root:arroba "$ARROBA_KERNEL_CAPABILITY_DIR"
  chown root:arroba-gateway "$ARROBA_GATEWAY_CAPABILITY_DIR"
  chmod 711 "$ARROBA_CAPABILITY_ROOT"
  chmod 700 "$ARROBA_CAPABILITY_STAGING_DIR"
  chmod 1730 "$ARROBA_KERNEL_CAPABILITY_DIR" "$ARROBA_GATEWAY_CAPABILITY_DIR"
}

write_private_capability_file() {
  local path="$1"
  local owner="$2"
  local value="$3"
  local owner_uid
  local owner_gid
  owner_uid="$(id -u "$owner")"
  owner_gid="$(id -g "$owner")"
  printf '%s' "$value" | node -e '
    const crypto = require("node:crypto")
    const fs = require("node:fs")
    const path = require("node:path")
    const [stagingDirectory, destination, ownerUid, ownerGid] = process.argv.slice(1)
    const temporary = path.join(
      stagingDirectory,
      `.${path.basename(destination)}.${process.pid}.${crypto.randomBytes(16).toString("hex")}`,
    )
    const flags = fs.constants.O_WRONLY
      | fs.constants.O_CREAT
      | fs.constants.O_EXCL
      | fs.constants.O_NOFOLLOW
    let descriptor
    try {
      descriptor = fs.openSync(temporary, flags, 0o600)
      fs.writeFileSync(descriptor, fs.readFileSync(0))
      fs.fsyncSync(descriptor)
      fs.fchownSync(descriptor, Number(ownerUid), Number(ownerGid))
      fs.fchmodSync(descriptor, 0o600)
      fs.closeSync(descriptor)
      descriptor = undefined
      fs.renameSync(temporary, destination)
      const directoryDescriptor = fs.openSync(path.dirname(destination), fs.constants.O_RDONLY | fs.constants.O_DIRECTORY)
      fs.fsyncSync(directoryDescriptor)
      fs.closeSync(directoryDescriptor)
    } catch (error) {
      if (descriptor !== undefined) fs.closeSync(descriptor)
      try { fs.unlinkSync(temporary) } catch {}
      throw error
    }
  ' "$ARROBA_CAPABILITY_STAGING_DIR" "$path" "$owner_uid" "$owner_gid"
}

require_private_capability_file() {
  local path="$1"
  local owner="$2"
  local label="$3"
  if [[ -z "$path" ]]; then
    echo "publication $label file is required" >&2
    return 70
  fi
  if [[ -L "$path" || ! -f "$path" ]]; then
    echo "publication $label file must be a regular non-symlink file: $path" >&2
    return 70
  fi
  if [[ "$(id -u)" -eq 0 ]]; then
    if ! /usr/sbin/runuser -u "$owner" -- test -r "$path"; then
      echo "publication $label file is not readable by $owner: $path" >&2
      return 70
    fi
  elif [[ ! -r "$path" ]]; then
    echo "publication $label file is not readable: $path" >&2
    return 70
  fi
}

read_bootstrap_capability_file() {
  local path="$1"
  local label="$2"
  node -e '
    const fs = require("node:fs")
    const [source, label] = process.argv.slice(1)
    let descriptor
    try {
      descriptor = fs.openSync(source, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW)
      const metadata = fs.fstatSync(descriptor)
      if (!metadata.isFile() || (metadata.mode & 0o077) !== 0) {
        throw new Error(`publication ${label} file must be a private regular file`)
      }
      process.stdout.write(fs.readFileSync(descriptor, "utf8"))
    } finally {
      if (descriptor !== undefined) fs.closeSync(descriptor)
    }
  ' "$path" "$label"
}

clear_exported_environment() {
  local name
  while IFS= read -r name; do
    export -n "$name" 2>/dev/null || true
  done < <(compgen -e)
}

export_if_set() {
  local name="$1"
  if declare -p "$name" >/dev/null 2>&1; then
    export "$name"
  fi
}

run_as_arroba() {
  if [[ "$(id -u)" -eq 0 ]]; then
    exec /usr/sbin/runuser -u arroba -- env HOME="$HOME" USER=arroba TMPDIR="$ARROBA_KERNEL_TMPDIR" "$@"
  fi
  export TMPDIR="$ARROBA_KERNEL_TMPDIR"
  exec "$@"
}

launch_kernel_as_arroba() {
  local kernel_local_auth_file="$1"
  shift
  require_private_capability_file "$kernel_local_auth_file" arroba "kernel local auth token"
  clear_exported_environment
  local name
  # DaemonConfig, provider harness configuration, and provider network trust inputs.
  for name in \
      LANG \
      LANGUAGE \
      LC_ALL \
      LC_CTYPE \
      SSL_CERT_FILE \
      SSL_CERT_DIR \
      NODE_EXTRA_CA_CERTS \
      HTTP_PROXY \
      HTTPS_PROXY \
      ALL_PROXY \
      NO_PROXY \
      http_proxy \
      https_proxy \
      all_proxy \
      no_proxy \
      NODE_ENV \
      NPM_CONFIG_CACHE \
      XDG_CACHE_HOME \
      ARROBA_ACCEPT_REMOTE_LEASES \
      ARROBA_CAPABILITY_ISOLATION_ROOT \
      ARROBA_CLAUDE_CONFIG \
      ARROBA_CLAUDE_HEADLESS_DEBUG \
      ARROBA_CLAUDE_TURN_STALL_TIMEOUT_MS \
      ARROBA_CLOUD_RELAY_CONFIG_JSON \
      ARROBA_CONFIG_DIR \
      ARROBA_CONNECTOR_ADAPTER_BUNDLED_DIR \
      ARROBA_DATA_DIR \
      ARROBA_DAEMON_ALIAS \
      ARROBA_DAEMON_ID \
      ARROBA_DAEMON_SOCKET \
      ARROBA_HARNESS_OUTPUT_TIMEOUT_MS \
      ARROBA_HARNESS_PROVIDER_LAUNCH_TIMEOUT_MS \
      ARROBA_HOME \
      ARROBA_RUNTIME_DIR \
      ARROBA_SESSION_HISTORY_DIR \
      ARROBA_SESSION_HISTORY_READ_DELAY_MS \
      ARROBA_WORKSPACE_DIR \
      ARROBA_PUBLICATION_RUNTIME_STATE_DIR \
      ARROBA_PUBLICATION_RUNTIME_ROOT \
      ARROBA_PUBLICATION_PACKAGE \
      ARROBA_PUBLICATION_CONFIG \
      ARROBA_PUBLICATION_ID \
      ARROBA_PUBLICATION_SESSION_ID \
      ARROBA_PUBLICATION_WORKFLOW \
      ARROBA_PUBLICATION_ENDPOINT \
      ARROBA_PUBLICATION_ROUTE \
      ARROBA_PUBLICATION_MODE \
      ARROBA_PUBLICATION_HOOK_ID \
      ARROBA_PUBLICATION_RUNTIME_WORKSPACE \
      ARROBA_PROVIDER_DEV_STUB \
      ARROBA_PROVIDER_PROCESS_IDLE_TTL_MS \
      ARROBA_PROVIDER_PROCESS_ORPHAN_TTL_MS \
      ARROBA_CODEX_BIN \
      ARROBA_CLAUDE_BIN \
      ARROBA_OPENCODE_BIN \
      ARROBA_OPENCODE_PORT \
      ARROBA_KERNEL_HOST \
      ARROBA_KERNEL_PORT \
      ARROBA_KERNEL_QUEUE_CAPACITY \
      ARROBA_KERNEL_WRITE_DELAY_MS \
      ARROBA_MACHINE_ALIAS \
      ARROBA_MACHINE_ID \
      ARROBA_MCP_HOST \
      ARROBA_MCP_PORT \
      ARROBA_OS_NAME \
      ARROBA_RELAY_HEARTBEAT_MS \
      ARROBA_RELAY_REQUEST_TIMEOUT_MS \
      ARROBA_RELAY_TOKEN \
      ARROBA_RELAY_URL \
      ARROBA_LOG_LEVEL \
      CLAUDE_HOME \
      CODEX_HOME \
      OPENCODE_CONFIG \
      OPENCODE_CONFIG_DIR \
      OPENCODE_DATA_HOME; do
    export_if_set "$name"
  done
  export HOME=/home/arroba
  export USER=arroba
  export LOGNAME=arroba
  export PATH=/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin
  export ARROBA_KERNEL_HOST="${ARROBA_KERNEL_HOST:-127.0.0.1}"
  export ARROBA_KERNEL_PORT="${ARROBA_KERNEL_PORT:-43118}"
  export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"
  export XDG_CONFIG_HOME="$HOME/.config"
  export XDG_DATA_HOME="$HOME/.local/share"
  export XDG_STATE_HOME="$HOME/.local/state"
  export NPM_CONFIG_CACHE="${NPM_CONFIG_CACHE:-$HOME/.cache/npm}"
  export TMPDIR="$ARROBA_KERNEL_TMPDIR"
  export ARROBA_LOG_DIR="$HOME/.local/state/arroba/logs"
  export ARROBA_KERNEL_LOCAL_AUTH_TOKEN_FILE="$kernel_local_auth_file"
  cd "$ARROBA_WORKSPACE_DIR"
  if [[ "$(id -u)" -eq 0 ]]; then
    exec /usr/sbin/runuser -u arroba --preserve-environment -- arroba-kernel "$@"
  fi
  if [[ "$(id -u)" -ne 1001 ]]; then
    echo "publication kernel must start as root or uid 1001" >&2
    exit 70
  fi
  exec arroba-kernel "$@"
}

spawn_kernel_as_arroba() {
  launch_kernel_as_arroba "$@" &
}

first_unsafe_tree_path() {
  find -P "$1" \( -type l -o \( ! -type d ! -type f \) \) -print -quit
}

validate_credential_destination() {
  if [[ -L "$HOME" || ! -d "$HOME" ]]; then
    echo "publication credential home must be a regular directory: $HOME" >&2
    return 70
  fi
  local entry
  local path
  local unsafe_path
  for entry in .codex .claude .claude.json .config .local; do
    path="$HOME/$entry"
    if [[ ! -e "$path" && ! -L "$path" ]]; then
      continue
    fi
    unsafe_path="$(first_unsafe_tree_path "$path")"
    if [[ -n "$unsafe_path" ]]; then
      echo "publication credential destination contains an unsafe path: $unsafe_path" >&2
      return 70
    fi
  done
}

import_credential_profile() {
  local profile_dir="$1"
  if [[ ! -d "$profile_dir" ]]; then
    return
  fi

  local unsafe_path
  unsafe_path="$(first_unsafe_tree_path "$profile_dir")"
  if [[ -n "$unsafe_path" ]]; then
    echo "publication credential profile contains an unsafe path: $unsafe_path" >&2
    return 70
  fi

  if [[ -d "$profile_dir/home" ]]; then
    shopt -s dotglob nullglob
    local source
    for source in "$profile_dir/home"/*; do
      case "${source##*/}" in
        .codex|.claude|.claude.json|.config|.local)
          cp -a -- "$source" "$HOME/"
          ;;
        *)
          echo "publication credential profile contains an unsupported home entry: $source" >&2
          return 70
          ;;
      esac
    done
    shopt -u dotglob nullglob
  else
    local entry
    for entry in .codex .claude .claude.json .config .local; do
      if [[ -e "$profile_dir/$entry" ]]; then
        cp -a -- "$profile_dir/$entry" "$HOME/"
      fi
    done
  fi

  mkdir -p "$ARROBA_CONFIG_DIR" "$ARROBA_DATA_DIR" "$ARROBA_RUNTIME_DIR" "$ARROBA_SESSION_HISTORY_DIR" "$HOME/.cache"
}

import_provider_credentials() {
  import_credential_profile "${ARROBA_PROVIDER_CREDENTIALS_DIR:-/home/arroba/.provider-credentials}"
}

import_credential_bindings() {
  local bindings_root="${ARROBA_CREDENTIAL_BINDINGS_ROOT:-}"
  if [[ -z "$bindings_root" ]]; then
    return
  fi
  if [[ -L "$bindings_root" ]]; then
    echo "publication credential bindings root must not be a symlink: $bindings_root" >&2
    return 70
  fi
  if [[ ! -d "$bindings_root" ]]; then
    return
  fi
  shopt -s nullglob
  local profile_dir
  for profile_dir in "$bindings_root"/*; do
    if [[ -d "$profile_dir" ]]; then
      import_credential_profile "$profile_dir"
    fi
  done
  shopt -u nullglob
}

validate_credential_destination
import_provider_credentials
import_credential_bindings
if [[ "$(id -u)" -eq 0 ]]; then
  chown arroba:arroba "$HOME"
fi
chmod 700 "$HOME"
chown -R arroba:arroba \
  "$ARROBA_CONFIG_DIR" \
  "$ARROBA_DATA_DIR" \
  "$ARROBA_RUNTIME_DIR" \
  "$ARROBA_SESSION_HISTORY_DIR" \
  "$ARROBA_PUBLICATION_RUNTIME_STATE_DIR" \
  "$ARROBA_KERNEL_TMPDIR" \
  "$HOME/.cache" \
  "$HOME/.codex" \
  "$HOME/.claude" \
  "$HOME/.claude.json" \
  "$HOME/.config" \
  "$HOME/.local" 2>/dev/null || true
if [[ "$(id -u)" -eq 0 ]]; then
  chown -R arroba:arroba "$ARROBA_WORKSPACE_DIR"
fi
chmod 700 "$ARROBA_WORKSPACE_DIR"
chmod -R go-rwx "$ARROBA_WORKSPACE_DIR" "$ARROBA_KERNEL_TMPDIR" "$HOME/.cache" "$HOME/.codex" "$HOME/.claude" "$HOME/.claude.json" "$HOME/.config" "$HOME/.local" 2>/dev/null || true

if [[ "$(id -u)" -eq 0 ]]; then
  mkdir -p \
    "$ARROBA_ACTION_HOME" \
    "$ARROBA_ACTION_TMPDIR" \
    "$ARROBA_GATEWAY_HOME/.cache" \
    "$ARROBA_GATEWAY_HOME/.config" \
    "$ARROBA_GATEWAY_HOME/.local/state" \
    "$ARROBA_GATEWAY_TMPDIR" \
    "$ARROBA_GATEWAY_RUNTIME_STATE_DIR"
  chown -R arroba-action:arroba-action "$ARROBA_ACTION_HOME"
  chown -R arroba-gateway:arroba-gateway "$ARROBA_GATEWAY_HOME"
  chmod 700 \
    "$ARROBA_ACTION_HOME" \
    "$ARROBA_ACTION_TMPDIR" \
    "$ARROBA_GATEWAY_HOME" \
    "$ARROBA_GATEWAY_TMPDIR"
fi

if [[ -z "${ARROBA_PUBLICATION_PACKAGE:-}" && -f /publication/publication.json ]]; then
  export ARROBA_PUBLICATION_PACKAGE=/publication
fi

gateway() {
  local kernel_local_auth_file="$1"
  local publication_audit_file="$2"
  shift 2
  require_private_capability_file "$kernel_local_auth_file" arroba-gateway "kernel local auth token"
  if [[ -n "$publication_audit_file" ]]; then
    require_private_capability_file "$publication_audit_file" arroba-gateway "publication audit capability"
  fi
  clear_exported_environment
  local name
  # Server publication configuration, self-hosted Cloud profile, and outbound trust inputs.
  for name in \
      LANG \
      LANGUAGE \
      LC_ALL \
      LC_CTYPE \
      SSL_CERT_FILE \
      SSL_CERT_DIR \
      NODE_EXTRA_CA_CERTS \
      HTTP_PROXY \
      HTTPS_PROXY \
      ALL_PROXY \
      NO_PROXY \
      http_proxy \
      https_proxy \
      all_proxy \
      no_proxy \
      NODE_ENV \
      HOST \
      PORT \
      ARROBA_KERNEL_HOST \
      ARROBA_KERNEL_PORT \
      ARROBA_KERNEL_URL \
      ARROBA_PUBLICATION_PACKAGE \
      ARROBA_PUBLICATION_CONFIG \
      ARROBA_PUBLICATION_ID \
      ARROBA_PUBLICATION_SESSION_ID \
      ARROBA_PUBLICATION_WORKFLOW \
      ARROBA_PUBLICATION_ENDPOINT \
      ARROBA_PUBLICATION_ROUTE \
      ARROBA_PUBLICATION_MODE \
      ARROBA_PUBLICATION_HOOK_ID \
      ARROBA_PUBLICATION_RUNTIME_WORKSPACE \
      ARROBA_PUBLICATION_HOST \
      ARROBA_PUBLICATION_PORT \
      ARROBA_PUBLICATION_TLS_ENABLED \
      ARROBA_PUBLICATION_TLS_KEY_FILE \
      ARROBA_PUBLICATION_TLS_CERT_FILE \
      ARROBA_PUBLICATION_CLOUD_API_URL \
      ARROBA_PUBLICATION_CLOUD_ACCOUNT_ID \
      ARROBA_PUBLICATION_CLOUD_SESSION_TOKEN \
      ARROBA_PUBLICATION_CLOUD_DEPLOYMENT_ID \
      ARROBA_PROVIDER_DEV_STUB \
      ARROBA_CODEX_BIN \
      ARROBA_CLAUDE_BIN \
      ARROBA_OPENCODE_BIN \
      ARROBA_LOG_LEVEL; do
    export_if_set "$name"
  done
  export HOME="$ARROBA_GATEWAY_HOME"
  export USER=arroba-gateway
  export LOGNAME=arroba-gateway
  export PATH=/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin
  export HOST="${HOST:-0.0.0.0}"
  export PORT="${PORT:-3000}"
  export ARROBA_KERNEL_HOST="${ARROBA_KERNEL_HOST:-127.0.0.1}"
  export ARROBA_KERNEL_PORT="${ARROBA_KERNEL_PORT:-43118}"
  if [[ -n "${ARROBA_KERNEL_URL:-}" ]]; then
    export ARROBA_KERNEL_URL
  fi
  export ARROBA_PUBLICATION_RUNTIME_STATE_DIR="$ARROBA_GATEWAY_RUNTIME_STATE_DIR"
  export ARROBA_LOG_DIR="$ARROBA_GATEWAY_HOME/.local/state/arroba/logs"
  export XDG_CACHE_HOME="$ARROBA_GATEWAY_HOME/.cache"
  export XDG_CONFIG_HOME="$ARROBA_GATEWAY_HOME/.config"
  export XDG_STATE_HOME="$ARROBA_GATEWAY_HOME/.local/state"
  export NPM_CONFIG_CACHE="$ARROBA_GATEWAY_HOME/.cache/npm"
  export TMPDIR="$ARROBA_GATEWAY_TMPDIR"
  export ARROBA_KERNEL_LOCAL_AUTH_TOKEN_FILE="$kernel_local_auth_file"
  if [[ -n "$publication_audit_file" ]]; then
    export ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL_FILE="$publication_audit_file"
  fi
  cd /publication
  if [[ "$(id -u)" -eq 0 ]]; then
    exec /usr/sbin/runuser -u arroba-gateway --preserve-environment -- \
      node /opt/arroba/apps/server/dist/index.js "$@"
  fi
  if [[ "$(id -u)" -ne 1003 ]]; then
    echo "publication gateway must start as root or uid 1003" >&2
    exit 70
  fi
  exec node /opt/arroba/apps/server/dist/index.js "$@"
}

ACTION_SERVER_PID=""
KERNEL_PID=""
GATEWAY_PID=""

cleanup_standalone_children() {
  local pid
  trap - INT TERM
  for pid in "$GATEWAY_PID" "$ACTION_SERVER_PID" "$KERNEL_PID"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  for pid in "$GATEWAY_PID" "$ACTION_SERVER_PID" "$KERNEL_PID"; do
    if [[ -n "$pid" ]]; then
      wait "$pid" 2>/dev/null || true
    fi
  done
}

start_action_server() {
  if [[ ! -f /publication/app/actions.mjs ]]; then
    return
  fi
  if [[ "$(id -u)" -eq 0 ]]; then
    (
      cd /publication/app
      exec /usr/sbin/runuser -u arroba-action -- env -i \
        HOME="$ARROBA_ACTION_HOME" \
        USER=arroba-action \
        LOGNAME=arroba-action \
        PATH=/usr/local/bin:/usr/bin:/bin \
        NODE_ENV="${NODE_ENV:-production}" \
        PORT="${ARROBA_AGENT_APP_ACTIONS_PORT:-33119}" \
        TMPDIR="$ARROBA_ACTION_TMPDIR" \
        node /publication/app/actions.mjs
    ) &
  elif [[ "$(id -u)" -eq 1002 ]]; then
    (
      cd /publication/app
      exec env -i \
        HOME="$ARROBA_ACTION_HOME" \
        USER=arroba-action \
        LOGNAME=arroba-action \
        PATH=/usr/local/bin:/usr/bin:/bin \
        NODE_ENV="${NODE_ENV:-production}" \
        PORT="${ARROBA_AGENT_APP_ACTIONS_PORT:-33119}" \
        TMPDIR="$ARROBA_ACTION_TMPDIR" \
        node /publication/app/actions.mjs
    ) &
  else
    echo "publication actions must start as root or uid 1002" >&2
    return 70
  fi
  ACTION_SERVER_PID="$!"
}

standalone() {
  local kernel_local_auth_token
  local publication_audit_url
  local publication_audit_source_file
  local publication_audit_file=""
  kernel_local_auth_token="$(generate_kernel_local_auth_token)"
  publication_audit_url="${ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL:-}"
  publication_audit_source_file="${ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL_FILE:-}"
  if [[ -n "$publication_audit_url" && -n "$publication_audit_source_file" ]]; then
    echo "publication audit URL and URL file cannot both be configured" >&2
    return 70
  fi
  if [[ -n "$publication_audit_source_file" ]]; then
    publication_audit_url="$(read_bootstrap_capability_file "$publication_audit_source_file" "audit URL")"
  fi
  prepare_capability_directories
  write_private_capability_file "$ARROBA_KERNEL_AUTH_FILE" arroba "$kernel_local_auth_token"
  write_private_capability_file "$ARROBA_GATEWAY_AUTH_FILE" arroba-gateway "$kernel_local_auth_token"
  if [[ -n "$publication_audit_url" ]]; then
    write_private_capability_file "$ARROBA_GATEWAY_AUDIT_FILE" arroba-gateway "$publication_audit_url"
    publication_audit_file="$ARROBA_GATEWAY_AUDIT_FILE"
  fi
  unset ARROBA_KERNEL_LOCAL_AUTH_TOKEN \
    ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL \
    ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL_FILE
  kernel_local_auth_token=""
  publication_audit_url=""
  publication_audit_source_file=""
  trap cleanup_standalone_children EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  spawn_kernel_as_arroba "$ARROBA_KERNEL_AUTH_FILE"
  KERNEL_PID="$!"
  start_action_server

  node /usr/local/bin/arroba-wait-for-tcp.mjs "${ARROBA_KERNEL_HOST:-127.0.0.1}" "${ARROBA_KERNEL_PORT:-43118}" 20000
  gateway "$ARROBA_GATEWAY_AUTH_FILE" "$publication_audit_file" "$@" &
  GATEWAY_PID="$!"
  local gateway_status=0
  wait "$GATEWAY_PID" || gateway_status="$?"
  GATEWAY_PID=""
  return "$gateway_status"
}

case "${1:-standalone}" in
  standalone)
    shift || true
    standalone "$@"
    ;;
  gateway)
    shift || true
    gateway "${ARROBA_KERNEL_LOCAL_AUTH_TOKEN_FILE:-}" "${ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL_FILE:-}" "$@"
    ;;
  kernel)
    shift || true
    launch_kernel_as_arroba "${ARROBA_KERNEL_LOCAL_AUTH_TOKEN_FILE:-}" "$@"
    ;;
  actions)
    shift || true
    start_action_server
    wait "$ACTION_SERVER_PID"
    ;;
  bash|sh)
    run_as_arroba "$@"
    ;;
  *)
    run_as_arroba "$@"
    ;;
esac
