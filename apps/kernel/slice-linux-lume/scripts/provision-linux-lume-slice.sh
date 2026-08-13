#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

SLICE_NAME="${CHARIOX_SLICE_NAME:-chariox-slice-linux-lume}"
SLICE_IMAGE="${CHARIOX_SLICE_IMAGE:-ubuntu-noble-vanilla:latest}"
SLICE_USER="${CHARIOX_SLICE_USER:-lume}"
SLICE_PASSWORD="${CHARIOX_SLICE_PASSWORD:-lume}"
SLICE_CPU="${CHARIOX_SLICE_CPU:-4}"
SLICE_MEMORY="${CHARIOX_SLICE_MEMORY:-8GB}"
SLICE_DISK_SIZE="${CHARIOX_SLICE_DISK_SIZE:-50GB}"
SLICE_INSTALL_CHROMIUM="${CHARIOX_SLICE_INSTALL_CHROMIUM:-1}"
SLICE_START_PROVIDER_SERVERS="${CHARIOX_SLICE_START_PROVIDER_SERVERS:-1}"
SLICE_REMOTE_ROOT="${CHARIOX_SLICE_REMOTE_ROOT:-/home/$SLICE_USER/chariox-slice}"
SLICE_CODEX_PORT="${CHARIOX_SLICE_CODEX_PORT:-43252}"
SLICE_OPENCODE_PORT="${CHARIOX_SLICE_OPENCODE_PORT:-43140}"
SLICE_KERNEL_PORT="${CHARIOX_SLICE_KERNEL_PORT:-43119}"
SLICE_RELAY_PORT="${CHARIOX_SLICE_RELAY_PORT:-43130}"

export PATH="$HOME/.local/bin:$PATH"

log() {
  printf '[slice-linux-lume] %s\n' "$*" >&2
}

fail() {
  printf '[slice-linux-lume] error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<EOF
Usage: $(basename "$0") [provision|status|start-runtime|start-providers]

Environment overrides:
  CHARIOX_SLICE_NAME=$SLICE_NAME
  CHARIOX_SLICE_IMAGE=$SLICE_IMAGE
  CHARIOX_SLICE_CPU=$SLICE_CPU
  CHARIOX_SLICE_MEMORY=$SLICE_MEMORY
  CHARIOX_SLICE_DISK_SIZE=$SLICE_DISK_SIZE
  CHARIOX_SLICE_START_PROVIDER_SERVERS=$SLICE_START_PROVIDER_SERVERS

Default action is provision.
EOF
}

require_host() {
  [[ "$(uname -s)" == "Darwin" ]] || fail "this Lume slice provisioner currently targets macOS hosts"
  [[ "$(uname -m)" == "arm64" ]] || fail "Lume slices require Apple Silicon"
  command -v curl >/dev/null || fail "curl is required"
  command -v jq >/dev/null || fail "jq is required"
  command -v expect >/dev/null || fail "expect is required"
  command -v ssh >/dev/null || fail "ssh is required"
  command -v scp >/dev/null || fail "scp is required"
  command -v tar >/dev/null || fail "tar is required"
}

ensure_lume() {
  if command -v lume >/dev/null; then
    log "using Lume: $(lume --version 2>/dev/null || printf unknown)"
    return
  fi

  log "installing Lume"
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/trycua/cua/main/libs/lume/scripts/install.sh)"
  hash -r
  command -v lume >/dev/null || fail "Lume install finished but lume is not on PATH; add ~/.local/bin to PATH"
}

vm_exists() {
  lume get "$SLICE_NAME" --format json >/dev/null 2>&1
}

ensure_vm() {
  if vm_exists; then
    log "VM $SLICE_NAME already exists"
  else
    log "pulling $SLICE_IMAGE as $SLICE_NAME"
    lume pull "$SLICE_IMAGE" "$SLICE_NAME"
  fi

  log "setting VM resources cpu=$SLICE_CPU memory=$SLICE_MEMORY disk=$SLICE_DISK_SIZE"
  lume set "$SLICE_NAME" --cpu "$SLICE_CPU" --memory "$SLICE_MEMORY" --disk-size "$SLICE_DISK_SIZE" || true

  log "starting $SLICE_NAME headlessly"
  lume run "$SLICE_NAME" --no-display || true
}

slice_ip() {
  lume get "$SLICE_NAME" --format json | jq -r '.ip // .network.ip // empty'
}

wait_for_ip() {
  local ip
  for _ in $(seq 1 120); do
    ip="$(slice_ip || true)"
    if [[ -n "$ip" && "$ip" != "null" ]]; then
      printf '%s\n' "$ip"
      return
    fi
    sleep 2
  done
  fail "timed out waiting for VM IP"
}

expect_ssh() {
  local ip="$1"
  local remote_command="$2"
  SSH_TARGET="$SLICE_USER@$ip" \
  SSH_REMOTE_COMMAND="$remote_command" \
  CHARIOX_SLICE_PASSWORD="$SLICE_PASSWORD" \
  expect <<'EXPECT'
set timeout -1
spawn ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR $env(SSH_TARGET) $env(SSH_REMOTE_COMMAND)
expect {
  -re "yes/no" { send "yes\r"; exp_continue }
  -re "(P|p)assword:" { send "$env(CHARIOX_SLICE_PASSWORD)\r"; exp_continue }
  eof {
    catch wait result
    exit [lindex $result 3]
  }
}
EXPECT
}

expect_scp() {
  local ip="$1"
  local source="$2"
  local destination="$3"
  SCP_SOURCE="$source" \
  SCP_DESTINATION="$SLICE_USER@$ip:$destination" \
  CHARIOX_SLICE_PASSWORD="$SLICE_PASSWORD" \
  expect <<'EXPECT'
set timeout -1
spawn scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR $env(SCP_SOURCE) $env(SCP_DESTINATION)
expect {
  -re "yes/no" { send "yes\r"; exp_continue }
  -re "(P|p)assword:" { send "$env(CHARIOX_SLICE_PASSWORD)\r"; exp_continue }
  eof {
    catch wait result
    exit [lindex $result 3]
  }
}
EXPECT
}

wait_for_ssh() {
  local ip="$1"
  for _ in $(seq 1 90); do
    if expect_ssh "$ip" "true" >/dev/null 2>&1; then
      return
    fi
    sleep 2
  done
  fail "timed out waiting for SSH at $ip"
}

write_remote_provision_script() {
  local output="$1"
  cat >"$output" <<'REMOTE'
#!/usr/bin/env bash
set -Eeuo pipefail

log() {
  printf '[slice-guest] %s\n' "$*" >&2
}

need_sudo() {
  if command -v sudo >/dev/null; then
    sudo "$@"
  else
    "$@"
  fi
}

export DEBIAN_FRONTEND=noninteractive
REMOTE_ROOT="${CHARIOX_SLICE_REMOTE_ROOT:?}"

log "installing OS packages"
need_sudo apt-get update
need_sudo apt-get install -y \
  ca-certificates \
  curl \
  git \
  jq \
  bubblewrap \
  build-essential \
  pkg-config \
  libdbus-1-dev \
  libssl-dev \
  protobuf-compiler \
  screen \
  socat \
  lsof \
  unzip

if [[ "${CHARIOX_SLICE_INSTALL_CHROMIUM:-1}" == "1" ]]; then
  log "installing Chromium best-effort"
  need_sudo apt-get install -y chromium || need_sudo apt-get install -y chromium-browser || true
fi

if ! command -v node >/dev/null || [[ "$(node -p 'Number(process.versions.node.split(".")[0])' 2>/dev/null || echo 0)" -lt 22 ]]; then
  log "installing Node.js 22"
  curl -fsSL https://deb.nodesource.com/setup_22.x | need_sudo -E bash -
  need_sudo apt-get install -y nodejs
fi

log "installing provider CLIs"
need_sudo npm install -g @openai/codex opencode-ai @anthropic-ai/claude-code

if ! command -v cargo >/dev/null; then
  log "installing Rust toolchain"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi

# shellcheck disable=SC1090
source "$HOME/.cargo/env"

log "unpacking Chariox source"
rm -rf "$REMOTE_ROOT/src"
mkdir -p "$REMOTE_ROOT/src" "$REMOTE_ROOT/bin" "$REMOTE_ROOT/logs"
tar -xzf /tmp/chariox-slice-source.tgz -C "$REMOTE_ROOT/src"

log "building chariox-kernel"
cargo build --manifest-path "$REMOTE_ROOT/src/apps/kernel/Cargo.toml" --bin chariox-kernel

log "building chariox-relay"
cargo build --manifest-path "$REMOTE_ROOT/src/apps/relay/Cargo.toml" --bin chariox-relay

ln -sf "$REMOTE_ROOT/src/apps/kernel/target/debug/chariox-kernel" "$REMOTE_ROOT/bin/chariox-kernel"
ln -sf "$REMOTE_ROOT/src/apps/relay/target/debug/chariox-relay" "$REMOTE_ROOT/bin/chariox-relay"

cat >"$REMOTE_ROOT/start-runtime.sh" <<'EOF_RUNTIME'
#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOGS="$ROOT/logs"
mkdir -p "$LOGS"

screen -S chariox-slice-relay -X quit >/dev/null 2>&1 || true
screen -S chariox-slice-kernel -X quit >/dev/null 2>&1 || true

screen -dmS chariox-slice-relay "$ROOT/bin/chariox-relay"
sleep 1
screen -dmS chariox-slice-kernel env CHARIOX_KERNEL_PORT="${CHARIOX_SLICE_KERNEL_PORT:-43119}" "$ROOT/bin/chariox-kernel"

screen -ls | sed -n '/chariox-slice-/p'
EOF_RUNTIME
chmod +x "$REMOTE_ROOT/start-runtime.sh"

cat >"$REMOTE_ROOT/start-providers.sh" <<'EOF_PROVIDERS'
#!/usr/bin/env bash
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOGS="$ROOT/logs"
CODEX_PORT="${CHARIOX_SLICE_CODEX_PORT:-43252}"
OPENCODE_PORT="${CHARIOX_SLICE_OPENCODE_PORT:-43140}"
mkdir -p "$LOGS"

pkill -f "codex app-server.*$CODEX_PORT" >/dev/null 2>&1 || true
pkill -f "opencode serve.*$OPENCODE_PORT" >/dev/null 2>&1 || true

nohup codex app-server --listen "ws://127.0.0.1:$CODEX_PORT" >"$LOGS/codex-app-server.log" 2>&1 &
nohup opencode serve --hostname 127.0.0.1 --port "$OPENCODE_PORT" >"$LOGS/opencode-serve.log" 2>&1 &

sleep 3
pgrep -af "codex app-server|opencode serve" || true
EOF_PROVIDERS
chmod +x "$REMOTE_ROOT/start-providers.sh"

log "installed versions"
node --version
npm --version
codex --version || true
opencode --version || true
"$REMOTE_ROOT/bin/chariox-kernel" --version >/dev/null 2>&1 || true
"$REMOTE_ROOT/bin/chariox-relay" --version >/dev/null 2>&1 || true

if [[ "${CHARIOX_SLICE_START_PROVIDER_SERVERS:-1}" == "1" ]]; then
  log "starting provider servers"
  CHARIOX_SLICE_CODEX_PORT="${CHARIOX_SLICE_CODEX_PORT:-43252}" \
  CHARIOX_SLICE_OPENCODE_PORT="${CHARIOX_SLICE_OPENCODE_PORT:-43140}" \
  "$REMOTE_ROOT/start-providers.sh" || true
fi

log "done"
REMOTE
}

package_chariox_source() {
  local output="$1"
  tar -czf "$output" \
    -C "$REPO_ROOT" \
    --exclude='apps/kernel/target' \
    --exclude='apps/relay/target' \
    apps/kernel apps/relay
}

provision_guest() {
  local ip="$1"
  local tmp_dir source_tgz remote_script
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN

  source_tgz="$tmp_dir/chariox-slice-source.tgz"
  remote_script="$tmp_dir/provision-guest.sh"

  log "packaging Chariox kernel/relay source"
  package_chariox_source "$source_tgz"
  write_remote_provision_script "$remote_script"

  log "copying provision payload to $ip"
  expect_scp "$ip" "$source_tgz" "/tmp/chariox-slice-source.tgz"
  expect_scp "$ip" "$remote_script" "/tmp/provision-chariox-slice.sh"

  log "running guest provisioner"
  expect_ssh "$ip" "chmod +x /tmp/provision-chariox-slice.sh && CHARIOX_SLICE_REMOTE_ROOT='$SLICE_REMOTE_ROOT' CHARIOX_SLICE_INSTALL_CHROMIUM='$SLICE_INSTALL_CHROMIUM' CHARIOX_SLICE_START_PROVIDER_SERVERS='$SLICE_START_PROVIDER_SERVERS' CHARIOX_SLICE_CODEX_PORT='$SLICE_CODEX_PORT' CHARIOX_SLICE_OPENCODE_PORT='$SLICE_OPENCODE_PORT' CHARIOX_SLICE_KERNEL_PORT='$SLICE_KERNEL_PORT' CHARIOX_SLICE_RELAY_PORT='$SLICE_RELAY_PORT' /tmp/provision-chariox-slice.sh"
}

print_status() {
  local ip="$1"
  log "VM: $SLICE_NAME ip=$ip"
  expect_ssh "$ip" "set -e; echo '--- versions'; command -v codex && codex --version || true; command -v opencode && opencode --version || true; test -x '$SLICE_REMOTE_ROOT/bin/chariox-kernel' && echo '$SLICE_REMOTE_ROOT/bin/chariox-kernel' || true; test -x '$SLICE_REMOTE_ROOT/bin/chariox-relay' && echo '$SLICE_REMOTE_ROOT/bin/chariox-relay' || true; echo '--- processes'; pgrep -af 'chariox-kernel|chariox-relay|codex app-server|opencode serve' || true; echo '--- logs'; ls -1 '$SLICE_REMOTE_ROOT/logs' 2>/dev/null || true"
}

main() {
  local action="${1:-provision}"
  case "$action" in
    -h|--help|help)
      usage
      ;;
    provision)
      require_host
      ensure_lume
      ensure_vm
      local ip
      ip="$(wait_for_ip)"
      log "waiting for SSH at $ip"
      wait_for_ssh "$ip"
      provision_guest "$ip"
      print_status "$ip"
      ;;
    status)
      require_host
      ensure_lume
      print_status "$(wait_for_ip)"
      ;;
    start-runtime)
      require_host
      ensure_lume
      expect_ssh "$(wait_for_ip)" "CHARIOX_SLICE_KERNEL_PORT='$SLICE_KERNEL_PORT' '$SLICE_REMOTE_ROOT/start-runtime.sh'"
      ;;
    start-providers)
      require_host
      ensure_lume
      expect_ssh "$(wait_for_ip)" "CHARIOX_SLICE_CODEX_PORT='$SLICE_CODEX_PORT' CHARIOX_SLICE_OPENCODE_PORT='$SLICE_OPENCODE_PORT' '$SLICE_REMOTE_ROOT/start-providers.sh'"
      ;;
    *)
      usage
      fail "unknown action: $action"
      ;;
  esac
}

main "$@"
