#!/usr/bin/env bash
set -euo pipefail

mkdir -p "$ARROBA_CONFIG_DIR" "$ARROBA_DATA_DIR" "$ARROBA_RUNTIME_DIR" "$ARROBA_SESSION_HISTORY_DIR" "$HOME/.cache" /workspace

run_as_arroba() {
  if [[ "$(id -u)" -eq 0 ]]; then
    exec /usr/sbin/runuser -u arroba -- env HOME="$HOME" USER=arroba "$@"
  fi
  exec "$@"
}

spawn_as_arroba() {
  if [[ "$(id -u)" -eq 0 ]]; then
    /usr/sbin/runuser -u arroba -- env HOME="$HOME" USER=arroba "$@" &
  else
    "$@" &
  fi
}

import_provider_credentials() {
  local profile_dir="${ARROBA_PROVIDER_CREDENTIALS_DIR:-/home/arroba/.provider-credentials}"
  if [[ ! -d "$profile_dir" ]]; then
    return
  fi

  if [[ -d "$profile_dir/home" ]]; then
    shopt -s dotglob nullglob
    local source
    for source in "$profile_dir/home"/*; do
      cp -a "$source" "$HOME/"
    done
    shopt -u dotglob nullglob
  else
    local entry
    for entry in .codex .claude .claude.json .config .local; do
      if [[ -e "$profile_dir/$entry" ]]; then
        cp -a "$profile_dir/$entry" "$HOME/"
      fi
    done
  fi

  mkdir -p "$ARROBA_CONFIG_DIR" "$ARROBA_DATA_DIR" "$ARROBA_RUNTIME_DIR" "$ARROBA_SESSION_HISTORY_DIR" "$HOME/.cache"
}

import_provider_credentials
chown arroba:arroba "$HOME" 2>/dev/null || true
chmod 755 "$HOME" 2>/dev/null || true
chown -R arroba:arroba \
  "$ARROBA_CONFIG_DIR" \
  "$ARROBA_DATA_DIR" \
  "$ARROBA_RUNTIME_DIR" \
  "$ARROBA_SESSION_HISTORY_DIR" \
  /workspace \
  "$HOME/.cache" \
  "$HOME/.codex" \
  "$HOME/.claude" \
  "$HOME/.claude.json" \
  "$HOME/.config" \
  "$HOME/.local" 2>/dev/null || true
chmod -R go-rwx "$HOME/.cache" "$HOME/.codex" "$HOME/.claude" "$HOME/.claude.json" "$HOME/.config" "$HOME/.local" 2>/dev/null || true

if [[ -z "${ARROBA_PUBLICATION_PACKAGE:-}" && -f /publication/publication.json ]]; then
  export ARROBA_PUBLICATION_PACKAGE=/publication
fi

gateway() {
  run_as_arroba node /opt/arroba/apps/server/dist/index.js "$@"
}

kernel() {
  run_as_arroba arroba-kernel "$@"
}

standalone() {
  spawn_as_arroba arroba-kernel
  local kernel_pid=$!
  trap 'kill "$kernel_pid" 2>/dev/null || true' EXIT INT TERM

  node /usr/local/bin/arroba-wait-for-tcp.mjs "${ARROBA_KERNEL_HOST:-127.0.0.1}" "${ARROBA_KERNEL_PORT:-43118}" 20000
  gateway "$@"
}

case "${1:-standalone}" in
  standalone)
    shift || true
    standalone "$@"
    ;;
  gateway)
    shift || true
    gateway "$@"
    ;;
  kernel)
    shift || true
    kernel "$@"
    ;;
  bash|sh)
    run_as_arroba "$@"
    ;;
  *)
    run_as_arroba "$@"
    ;;
esac
