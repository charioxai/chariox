#!/usr/bin/env bash
set -euo pipefail
umask 077

export CHARIOX_PUBLICATION_RUNTIME_STATE_DIR="${CHARIOX_PUBLICATION_RUNTIME_STATE_DIR:-$CHARIOX_DATA_DIR/publication-runtime}"
export CHARIOX_WORKSPACE_DIR="${CHARIOX_WORKSPACE_DIR:-/workspace}"
readonly CHARIOX_ACTION_HOME=/home/chariox-action
readonly CHARIOX_GATEWAY_HOME=/home/chariox-gateway
readonly CHARIOX_KERNEL_TMPDIR="$HOME/.tmp"
readonly CHARIOX_ACTION_TMPDIR="$CHARIOX_ACTION_HOME/.tmp"
readonly CHARIOX_GATEWAY_TMPDIR="$CHARIOX_GATEWAY_HOME/.tmp"
readonly CHARIOX_CAPABILITY_ROOT=/run/chariox-publication-capabilities
readonly CHARIOX_CAPABILITY_STAGING_DIR="$CHARIOX_CAPABILITY_ROOT/.staging"
readonly CHARIOX_KERNEL_CAPABILITY_DIR="$CHARIOX_CAPABILITY_ROOT/kernel"
readonly CHARIOX_GATEWAY_CAPABILITY_DIR="$CHARIOX_CAPABILITY_ROOT/gateway"
readonly CHARIOX_KERNEL_AUTH_FILE="$CHARIOX_KERNEL_CAPABILITY_DIR/kernel-local-auth"
readonly CHARIOX_GATEWAY_AUTH_FILE="$CHARIOX_GATEWAY_CAPABILITY_DIR/kernel-local-auth"
readonly CHARIOX_GATEWAY_AUDIT_FILE="$CHARIOX_GATEWAY_CAPABILITY_DIR/publication-audit-url"
readonly CHARIOX_GATEWAY_CALLER_CLAIMS_FILE="$CHARIOX_GATEWAY_CAPABILITY_DIR/publication-caller-claims.json"
readonly CHARIOX_GATEWAY_RUNTIME_STATE_DIR="$CHARIOX_GATEWAY_HOME/.local/share/chariox/publication-runtime"

mkdir -p \
  "$CHARIOX_CONFIG_DIR" \
  "$CHARIOX_DATA_DIR" \
  "$CHARIOX_RUNTIME_DIR" \
  "$CHARIOX_SESSION_HISTORY_DIR" \
  "$CHARIOX_PUBLICATION_RUNTIME_STATE_DIR" \
  "$HOME/.cache" \
  "$CHARIOX_KERNEL_TMPDIR" \
  "$CHARIOX_WORKSPACE_DIR"

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
    "$CHARIOX_CAPABILITY_ROOT" \
    "$CHARIOX_CAPABILITY_STAGING_DIR" \
    "$CHARIOX_KERNEL_CAPABILITY_DIR" \
    "$CHARIOX_GATEWAY_CAPABILITY_DIR"; do
    if [[ -L "$path" ]]; then
      echo "publication capability directory must not be a symlink: $path" >&2
      return 70
    fi
  done
  mkdir -p \
    "$CHARIOX_CAPABILITY_STAGING_DIR" \
    "$CHARIOX_KERNEL_CAPABILITY_DIR" \
    "$CHARIOX_GATEWAY_CAPABILITY_DIR"
  chown root:root "$CHARIOX_CAPABILITY_ROOT" "$CHARIOX_CAPABILITY_STAGING_DIR"
  chown root:chariox "$CHARIOX_KERNEL_CAPABILITY_DIR"
  chown root:chariox-gateway "$CHARIOX_GATEWAY_CAPABILITY_DIR"
  chmod 711 "$CHARIOX_CAPABILITY_ROOT"
  chmod 700 "$CHARIOX_CAPABILITY_STAGING_DIR"
  chmod 1730 "$CHARIOX_KERNEL_CAPABILITY_DIR" "$CHARIOX_GATEWAY_CAPABILITY_DIR"
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
  ' "$CHARIOX_CAPABILITY_STAGING_DIR" "$path" "$owner_uid" "$owner_gid"
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

run_as_chariox() {
  if [[ "$(id -u)" -eq 0 ]]; then
    exec /usr/sbin/runuser -u chariox -- env HOME="$HOME" USER=chariox TMPDIR="$CHARIOX_KERNEL_TMPDIR" "$@"
  fi
  export TMPDIR="$CHARIOX_KERNEL_TMPDIR"
  exec "$@"
}

launch_kernel_as_chariox() {
  local kernel_local_auth_file="$1"
  shift
  require_private_capability_file "$kernel_local_auth_file" chariox "kernel local auth token"
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
      CHARIOX_ACCEPT_REMOTE_LEASES \
      CHARIOX_CAPABILITY_ISOLATION_ROOT \
      CHARIOX_CLAUDE_CONFIG \
      CHARIOX_CLAUDE_HEADLESS_DEBUG \
      CHARIOX_CLAUDE_TURN_STALL_TIMEOUT_MS \
      CHARIOX_CLOUD_RELAY_CONFIG_JSON \
      CHARIOX_CONFIG_DIR \
      CHARIOX_CONNECTOR_ADAPTER_BUNDLED_DIR \
      CHARIOX_DATA_DIR \
      CHARIOX_DAEMON_ALIAS \
      CHARIOX_DAEMON_ID \
      CHARIOX_DAEMON_SOCKET \
      CHARIOX_HARNESS_OUTPUT_TIMEOUT_MS \
      CHARIOX_HARNESS_PROVIDER_LAUNCH_TIMEOUT_MS \
      CHARIOX_HOME \
      CHARIOX_RUNTIME_DIR \
      CHARIOX_SESSION_HISTORY_DIR \
      CHARIOX_SESSION_HISTORY_READ_DELAY_MS \
      CHARIOX_WORKSPACE_DIR \
      CHARIOX_PUBLICATION_RUNTIME_STATE_DIR \
      CHARIOX_PUBLICATION_PROVIDER_ACCOUNT_BINDINGS \
      CHARIOX_PUBLICATION_RUNTIME_ROOT \
      CHARIOX_PUBLICATION_PACKAGE \
      CHARIOX_PUBLICATION_CONFIG \
      CHARIOX_PUBLICATION_ID \
      CHARIOX_PUBLICATION_SESSION_ID \
      CHARIOX_PUBLICATION_WORKFLOW \
      CHARIOX_PUBLICATION_ENDPOINT \
      CHARIOX_PUBLICATION_ROUTE \
      CHARIOX_PUBLICATION_MODE \
      CHARIOX_PUBLICATION_HOOK_ID \
      CHARIOX_PUBLICATION_RUNTIME_WORKSPACE \
      CHARIOX_PROVIDER_DEV_STUB \
      CHARIOX_PROVIDER_PROCESS_IDLE_TTL_MS \
      CHARIOX_PROVIDER_PROCESS_ORPHAN_TTL_MS \
      CHARIOX_CODEX_BIN \
      CHARIOX_CLAUDE_BIN \
      CHARIOX_OPENCODE_BIN \
      CHARIOX_OPENCODE_PORT \
      CHARIOX_KERNEL_HOST \
      CHARIOX_KERNEL_PORT \
      CHARIOX_KERNEL_QUEUE_CAPACITY \
      CHARIOX_KERNEL_WRITE_DELAY_MS \
      CHARIOX_MACHINE_ALIAS \
      CHARIOX_MACHINE_ID \
      CHARIOX_MCP_HOST \
      CHARIOX_MCP_PORT \
      CHARIOX_OS_NAME \
      CHARIOX_RELAY_HEARTBEAT_MS \
      CHARIOX_RELAY_REQUEST_TIMEOUT_MS \
      CHARIOX_RELAY_TOKEN \
      CHARIOX_RELAY_URL \
      CHARIOX_LOG_LEVEL \
      CLAUDE_HOME \
      CODEX_HOME \
      OPENCODE_CONFIG \
      OPENCODE_CONFIG_DIR \
      OPENCODE_DATA_HOME; do
    export_if_set "$name"
  done
  export HOME=/home/chariox
  export USER=chariox
  export LOGNAME=chariox
  export PATH=/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin
  export CHARIOX_KERNEL_HOST="${CHARIOX_KERNEL_HOST:-127.0.0.1}"
  export CHARIOX_KERNEL_PORT="${CHARIOX_KERNEL_PORT:-43118}"
  export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"
  export XDG_CONFIG_HOME="$HOME/.config"
  export XDG_DATA_HOME="$HOME/.local/share"
  export XDG_STATE_HOME="$HOME/.local/state"
  export NPM_CONFIG_CACHE="${NPM_CONFIG_CACHE:-$HOME/.cache/npm}"
  export TMPDIR="$CHARIOX_KERNEL_TMPDIR"
  export CHARIOX_LOG_DIR="$HOME/.local/state/chariox/logs"
  export CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE="$kernel_local_auth_file"
  cd "$CHARIOX_WORKSPACE_DIR"
  if [[ "$(id -u)" -eq 0 ]]; then
    exec /usr/sbin/runuser -u chariox --preserve-environment -- chariox-kernel "$@"
  fi
  if [[ "$(id -u)" -ne 1001 ]]; then
    echo "publication kernel must start as root or uid 1001" >&2
    exit 70
  fi
  exec chariox-kernel "$@"
}

spawn_kernel_as_chariox() {
  launch_kernel_as_chariox "$@" &
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

  mkdir -p "$CHARIOX_CONFIG_DIR" "$CHARIOX_DATA_DIR" "$CHARIOX_RUNTIME_DIR" "$CHARIOX_SESSION_HISTORY_DIR" "$HOME/.cache"
}

import_provider_credentials() {
  import_credential_profile "${CHARIOX_PROVIDER_CREDENTIALS_DIR:-/home/chariox/.provider-credentials}"
}

materialize_credential_bindings() {
  local source_root="${CHARIOX_CREDENTIAL_BINDINGS_SOURCE_ROOT:-}"
  local bindings_root="${CHARIOX_CREDENTIAL_BINDINGS_ROOT:-}"
  if [[ -z "$source_root" ]]; then
    return
  fi
  if [[ -z "$bindings_root" || "$bindings_root" != "$HOME/.credential-bindings" ]]; then
    echo "publication credential bindings destination is invalid" >&2
    return 70
  fi
  if [[ -L "$source_root" || ! -d "$source_root" ]]; then
    echo "publication credential bindings source must be a regular directory" >&2
    return 70
  fi
  if [[ -L "$bindings_root" ]]; then
    echo "publication credential bindings root must not be a symlink: $bindings_root" >&2
    return 70
  fi
  mkdir -p "$bindings_root"
  shopt -s nullglob
  local existing=("$bindings_root"/*)
  if (( ${#existing[@]} > 0 )); then
    echo "publication credential bindings destination must be empty" >&2
    return 70
  fi
  local count=0
  local source_profile
  for source_profile in "$source_root"/*; do
    local name="${source_profile##*/}"
    if [[ ! "$name" =~ ^[0-9]{3,}$ ]]; then
      echo "publication credential binding source name is invalid" >&2
      return 70
    fi
    if [[ -L "$source_profile" || ! -d "$source_profile" ]]; then
      echo "publication credential binding source must be a regular directory" >&2
      return 70
    fi
    local unsafe_path
    unsafe_path="$(first_unsafe_tree_path "$source_profile")"
    if [[ -n "$unsafe_path" ]]; then
      echo "publication credential binding source contains an unsafe path: $unsafe_path" >&2
      return 70
    fi
    count=$((count + 1))
    if (( count > 18 )); then
      echo "publication has too many credential bindings" >&2
      return 70
    fi
    cp -a -- "$source_profile" "$bindings_root/$name"
  done
  shopt -u nullglob
  chmod -R go-rwx "$bindings_root"
}

import_credential_bindings() {
  local bindings_root="${CHARIOX_CREDENTIAL_BINDINGS_ROOT:-}"
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
      if should_import_credential_binding_to_home "$profile_dir"; then
        import_credential_profile "$profile_dir"
      else
        local decision_status=$?
        if [[ "$decision_status" -ne 1 ]]; then
          return "$decision_status"
        fi
      fi
    fi
  done
  shopt -u nullglob
}

should_import_credential_binding_to_home() {
  local profile_dir="$1"
  node -e '
    const fs = require("node:fs")
    const path = require("node:path")
    const profileDir = process.argv[1]
    const configured = process.env.CHARIOX_PUBLICATION_PROVIDER_ACCOUNT_BINDINGS
    if (!configured) process.exit(0)
    let identity
    let manifest
    try {
      identity = JSON.parse(fs.readFileSync(path.join(profileDir, "profile.json"), "utf8"))
      manifest = JSON.parse(configured)
    } catch {
      process.exit(70)
    }
    if (identity.kind !== "provider") process.exit(0)
    const provider = String(identity.provider || "").toLowerCase()
    const profileId = String(identity.profileId || "")
    const selected = (Array.isArray(manifest.accounts) ? manifest.accounts : [])
      .filter((account) => String(account.provider || "").toLowerCase() === provider)
    const selectedIds = new Set(selected.map((account) => String(account.account_profile || "")))
    const defaults = Array.isArray(manifest.defaults) ? manifest.defaults : []
    const isDefault = defaults.some((account) => (
      String(account.provider || "").toLowerCase() === provider
      && String(account.account_profile || "") === profileId
    ))
    if (selectedIds.size > 1 && selectedIds.has(profileId) && !isDefault) process.exit(1)
    process.exit(0)
  ' "$profile_dir"
}

validate_credential_destination
import_provider_credentials
materialize_credential_bindings
import_credential_bindings
if [[ "$(id -u)" -eq 0 ]]; then
  chown chariox:chariox "$HOME"
fi
chmod 700 "$HOME"
chown -R chariox:chariox \
  "$CHARIOX_CONFIG_DIR" \
  "$CHARIOX_DATA_DIR" \
  "$CHARIOX_RUNTIME_DIR" \
  "$CHARIOX_SESSION_HISTORY_DIR" \
  "$CHARIOX_PUBLICATION_RUNTIME_STATE_DIR" \
  "$CHARIOX_KERNEL_TMPDIR" \
  "$HOME/.cache" \
  "$HOME/.codex" \
  "$HOME/.claude" \
  "$HOME/.claude.json" \
  "$HOME/.config" \
  "$HOME/.local" \
  "$HOME/.credential-bindings" 2>/dev/null || true
if [[ "$(id -u)" -eq 0 ]]; then
  chown -R chariox:chariox "$CHARIOX_WORKSPACE_DIR"
fi
chmod 700 "$CHARIOX_WORKSPACE_DIR"
chmod -R go-rwx "$CHARIOX_WORKSPACE_DIR" "$CHARIOX_KERNEL_TMPDIR" "$HOME/.cache" "$HOME/.codex" "$HOME/.claude" "$HOME/.claude.json" "$HOME/.config" "$HOME/.local" 2>/dev/null || true

if [[ "$(id -u)" -eq 0 ]]; then
  mkdir -p \
    "$CHARIOX_ACTION_HOME" \
    "$CHARIOX_ACTION_TMPDIR" \
    "$CHARIOX_GATEWAY_HOME/.cache" \
    "$CHARIOX_GATEWAY_HOME/.config" \
    "$CHARIOX_GATEWAY_HOME/.local/state" \
    "$CHARIOX_GATEWAY_TMPDIR" \
    "$CHARIOX_GATEWAY_RUNTIME_STATE_DIR"
  chown -R chariox-action:chariox-action "$CHARIOX_ACTION_HOME"
  chown -R chariox-gateway:chariox-gateway "$CHARIOX_GATEWAY_HOME"
  chmod 700 \
    "$CHARIOX_ACTION_HOME" \
    "$CHARIOX_ACTION_TMPDIR" \
    "$CHARIOX_GATEWAY_HOME" \
    "$CHARIOX_GATEWAY_TMPDIR"
fi

if [[ -z "${CHARIOX_PUBLICATION_PACKAGE:-}" && -f /publication/publication.json ]]; then
  export CHARIOX_PUBLICATION_PACKAGE=/publication
fi

gateway() {
  local kernel_local_auth_file="$1"
  local publication_audit_file="$2"
  local caller_claims_file="$3"
  shift 3
  require_private_capability_file "$kernel_local_auth_file" chariox-gateway "kernel local auth token"
  if [[ -n "$publication_audit_file" ]]; then
    require_private_capability_file "$publication_audit_file" chariox-gateway "publication audit capability"
  fi
  if [[ -n "$caller_claims_file" ]]; then
    require_private_capability_file "$caller_claims_file" chariox-gateway "caller claims configuration"
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
      CHARIOX_KERNEL_HOST \
      CHARIOX_KERNEL_PORT \
      CHARIOX_KERNEL_URL \
      CHARIOX_PUBLICATION_PACKAGE \
      CHARIOX_PUBLICATION_CONFIG \
      CHARIOX_PUBLICATION_ID \
      CHARIOX_PUBLICATION_SESSION_ID \
      CHARIOX_PUBLICATION_WORKFLOW \
      CHARIOX_PUBLICATION_ENDPOINT \
      CHARIOX_PUBLICATION_ROUTE \
      CHARIOX_PUBLICATION_MODE \
      CHARIOX_PUBLICATION_HOOK_ID \
      CHARIOX_PUBLICATION_RUNTIME_WORKSPACE \
      CHARIOX_PUBLICATION_HOST \
      CHARIOX_PUBLICATION_PORT \
      CHARIOX_PUBLICATION_TLS_ENABLED \
      CHARIOX_PUBLICATION_TLS_KEY_FILE \
      CHARIOX_PUBLICATION_TLS_CERT_FILE \
      CHARIOX_PUBLICATION_CLOUD_API_URL \
      CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID \
      CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN \
      CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID \
      CHARIOX_PROVIDER_DEV_STUB \
      CHARIOX_CODEX_BIN \
      CHARIOX_CLAUDE_BIN \
      CHARIOX_OPENCODE_BIN \
      CHARIOX_LOG_LEVEL; do
    export_if_set "$name"
  done
  export HOME="$CHARIOX_GATEWAY_HOME"
  export USER=chariox-gateway
  export LOGNAME=chariox-gateway
  export PATH=/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin
  export HOST="${HOST:-0.0.0.0}"
  export PORT="${PORT:-3000}"
  export CHARIOX_KERNEL_HOST="${CHARIOX_KERNEL_HOST:-127.0.0.1}"
  export CHARIOX_KERNEL_PORT="${CHARIOX_KERNEL_PORT:-43118}"
  if [[ -n "${CHARIOX_KERNEL_URL:-}" ]]; then
    export CHARIOX_KERNEL_URL
  fi
  export CHARIOX_PUBLICATION_RUNTIME_STATE_DIR="$CHARIOX_GATEWAY_RUNTIME_STATE_DIR"
  export CHARIOX_LOG_DIR="$CHARIOX_GATEWAY_HOME/.local/state/chariox/logs"
  export XDG_CACHE_HOME="$CHARIOX_GATEWAY_HOME/.cache"
  export XDG_CONFIG_HOME="$CHARIOX_GATEWAY_HOME/.config"
  export XDG_STATE_HOME="$CHARIOX_GATEWAY_HOME/.local/state"
  export NPM_CONFIG_CACHE="$CHARIOX_GATEWAY_HOME/.cache/npm"
  export TMPDIR="$CHARIOX_GATEWAY_TMPDIR"
  export CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE="$kernel_local_auth_file"
  if [[ -n "$publication_audit_file" ]]; then
    export CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE="$publication_audit_file"
  fi
  if [[ -n "$caller_claims_file" ]]; then
    export CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE="$caller_claims_file"
  fi
  cd /publication
  if [[ "$(id -u)" -eq 0 ]]; then
    exec /usr/sbin/runuser -u chariox-gateway --preserve-environment -- \
      node /opt/chariox/apps/server/dist/index.js "$@"
  fi
  if [[ "$(id -u)" -ne 1003 ]]; then
    echo "publication gateway must start as root or uid 1003" >&2
    exit 70
  fi
  exec node /opt/chariox/apps/server/dist/index.js "$@"
}

ACTION_SERVER_PID=""
KERNEL_PID=""
GATEWAY_PID=""
READINESS_PID=""
COMPLETED_CHILD_LABEL=""

cleanup_standalone_children() {
  local pid
  trap - INT TERM
  for pid in "$READINESS_PID" "$GATEWAY_PID" "$ACTION_SERVER_PID" "$KERNEL_PID"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  for pid in "$READINESS_PID" "$GATEWAY_PID" "$ACTION_SERVER_PID" "$KERNEL_PID"; do
    if [[ -n "$pid" ]]; then
      wait "$pid" 2>/dev/null || true
    fi
  done
}

record_standalone_child_exit() {
  local pid="$1"
  COMPLETED_CHILD_LABEL=""
  if [[ -n "$READINESS_PID" && "$pid" == "$READINESS_PID" ]]; then
    READINESS_PID=""
    COMPLETED_CHILD_LABEL="kernel readiness probe"
  elif [[ -n "$GATEWAY_PID" && "$pid" == "$GATEWAY_PID" ]]; then
    GATEWAY_PID=""
    COMPLETED_CHILD_LABEL="gateway"
  elif [[ -n "$ACTION_SERVER_PID" && "$pid" == "$ACTION_SERVER_PID" ]]; then
    ACTION_SERVER_PID=""
    COMPLETED_CHILD_LABEL="action server"
  elif [[ -n "$KERNEL_PID" && "$pid" == "$KERNEL_PID" ]]; then
    KERNEL_PID=""
    COMPLETED_CHILD_LABEL="kernel"
  fi
}

unexpected_standalone_child_status() {
  local label="$1"
  local status="$2"
  if [[ -z "$label" ]]; then
    echo "publication standalone supervisor lost track of a child process" >&2
    return 70
  fi
  echo "publication standalone child exited: $label (status $status)" >&2
  if [[ "$status" -eq 0 ]]; then
    return 70
  fi
  return "$status"
}

wait_for_kernel_readiness() {
  node /usr/local/bin/chariox-wait-for-tcp.mjs \
    "${CHARIOX_KERNEL_HOST:-127.0.0.1}" \
    "${CHARIOX_KERNEL_PORT:-43118}" \
    20000 &
  READINESS_PID="$!"
  local completed_pid=""
  local status=0
  local -a children=("$READINESS_PID" "$KERNEL_PID")
  if [[ -n "$ACTION_SERVER_PID" ]]; then
    children+=("$ACTION_SERVER_PID")
  fi
  if wait -n -p completed_pid "${children[@]}"; then
    status=0
  else
    status="$?"
  fi
  record_standalone_child_exit "$completed_pid"
  if [[ "$COMPLETED_CHILD_LABEL" == "kernel readiness probe" ]]; then
    return "$status"
  fi
  unexpected_standalone_child_status "$COMPLETED_CHILD_LABEL" "$status"
}

supervise_standalone_children() {
  local completed_pid=""
  local status=0
  local -a children=("$GATEWAY_PID" "$KERNEL_PID")
  if [[ -n "$ACTION_SERVER_PID" ]]; then
    children+=("$ACTION_SERVER_PID")
  fi
  if wait -n -p completed_pid "${children[@]}"; then
    status=0
  else
    status="$?"
  fi
  record_standalone_child_exit "$completed_pid"
  unexpected_standalone_child_status "$COMPLETED_CHILD_LABEL" "$status"
}

start_action_server() {
  if [[ ! -f /publication/app/actions.mjs ]]; then
    return
  fi
  if [[ "$(id -u)" -eq 0 ]]; then
    (
      cd /publication/app
      exec /usr/sbin/runuser -u chariox-action -- env -i \
        HOME="$CHARIOX_ACTION_HOME" \
        USER=chariox-action \
        LOGNAME=chariox-action \
        PATH=/usr/local/bin:/usr/bin:/bin \
        NODE_ENV="${NODE_ENV:-production}" \
        PORT="${CHARIOX_AGENT_APP_ACTIONS_PORT:-33119}" \
        TMPDIR="$CHARIOX_ACTION_TMPDIR" \
        node /publication/app/actions.mjs
    ) &
  elif [[ "$(id -u)" -eq 1002 ]]; then
    (
      cd /publication/app
      exec env -i \
        HOME="$CHARIOX_ACTION_HOME" \
        USER=chariox-action \
        LOGNAME=chariox-action \
        PATH=/usr/local/bin:/usr/bin:/bin \
        NODE_ENV="${NODE_ENV:-production}" \
        PORT="${CHARIOX_AGENT_APP_ACTIONS_PORT:-33119}" \
        TMPDIR="$CHARIOX_ACTION_TMPDIR" \
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
  local caller_claims_config=""
  local caller_claims_source_file
  local caller_claims_file=""
  local publication_audit_url
  local publication_audit_source_file
  local publication_audit_file=""
  kernel_local_auth_token="$(generate_kernel_local_auth_token)"
  publication_audit_url="${CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL:-}"
  publication_audit_source_file="${CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE:-}"
  caller_claims_source_file="${CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE:-}"
  if [[ -n "$publication_audit_url" && -n "$publication_audit_source_file" ]]; then
    echo "publication audit URL and URL file cannot both be configured" >&2
    return 70
  fi
  if [[ -n "$publication_audit_source_file" ]]; then
    publication_audit_url="$(read_bootstrap_capability_file "$publication_audit_source_file" "audit URL")"
  fi
  if [[ -n "$caller_claims_source_file" ]]; then
    caller_claims_config="$(read_bootstrap_capability_file "$caller_claims_source_file" "caller claims configuration")"
  fi
  prepare_capability_directories
  write_private_capability_file "$CHARIOX_KERNEL_AUTH_FILE" chariox "$kernel_local_auth_token"
  write_private_capability_file "$CHARIOX_GATEWAY_AUTH_FILE" chariox-gateway "$kernel_local_auth_token"
  if [[ -n "$publication_audit_url" ]]; then
    write_private_capability_file "$CHARIOX_GATEWAY_AUDIT_FILE" chariox-gateway "$publication_audit_url"
    publication_audit_file="$CHARIOX_GATEWAY_AUDIT_FILE"
  fi
  if [[ -n "$caller_claims_config" ]]; then
    write_private_capability_file "$CHARIOX_GATEWAY_CALLER_CLAIMS_FILE" chariox-gateway "$caller_claims_config"
    caller_claims_file="$CHARIOX_GATEWAY_CALLER_CLAIMS_FILE"
  fi
  unset CHARIOX_KERNEL_LOCAL_AUTH_TOKEN \
    CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL \
    CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE \
    CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE
  kernel_local_auth_token=""
  caller_claims_config=""
  caller_claims_source_file=""
  publication_audit_url=""
  publication_audit_source_file=""
  trap cleanup_standalone_children EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  spawn_kernel_as_chariox "$CHARIOX_KERNEL_AUTH_FILE"
  KERNEL_PID="$!"
  start_action_server

  wait_for_kernel_readiness
  gateway "$CHARIOX_GATEWAY_AUTH_FILE" "$publication_audit_file" "$caller_claims_file" "$@" &
  GATEWAY_PID="$!"
  supervise_standalone_children
}

case "${1:-standalone}" in
  standalone)
    shift || true
    standalone "$@"
    ;;
  gateway)
    shift || true
    gateway \
      "${CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE:-}" \
      "${CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE:-}" \
      "${CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE:-}" \
      "$@"
    ;;
  kernel)
    shift || true
    launch_kernel_as_chariox "${CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE:-}" "$@"
    ;;
  actions)
    shift || true
    start_action_server
    wait "$ACTION_SERVER_PID"
    ;;
  bash|sh)
    run_as_chariox "$@"
    ;;
  *)
    run_as_chariox "$@"
    ;;
esac
