#!/usr/bin/env bash
set -euo pipefail

mkdir -p "$ARROBA_CONFIG_DIR" "$ARROBA_DATA_DIR" "$ARROBA_RUNTIME_DIR" "$ARROBA_SESSION_HISTORY_DIR" /workspace

import_provider_credentials() {
  local profile_dir="${ARROBA_PROVIDER_CREDENTIALS_DIR:-/home/arroba/.provider-credentials}"
  if [[ ! -d "$profile_dir" ]]; then
    return
  fi

  if [[ -d "$profile_dir/home" ]]; then
    cp -a "$profile_dir/home/." "$HOME/"
  else
    local entry
    for entry in .codex .claude .claude.json .config .local; do
      if [[ -e "$profile_dir/$entry" ]]; then
        cp -a "$profile_dir/$entry" "$HOME/"
      fi
    done
  fi

  mkdir -p "$ARROBA_CONFIG_DIR" "$ARROBA_DATA_DIR" "$ARROBA_RUNTIME_DIR" "$ARROBA_SESSION_HISTORY_DIR"
  chmod -R go-rwx "$HOME/.codex" "$HOME/.claude" "$HOME/.claude.json" "$HOME/.config" "$HOME/.local" 2>/dev/null || true
}

import_provider_credentials

if [[ -z "${ARROBA_PUBLICATION_PACKAGE:-}" && -f /publication/publication.json ]]; then
  export ARROBA_PUBLICATION_PACKAGE=/publication
fi

gateway() {
  exec node /opt/arroba/apps/server/dist/index.js "$@"
}

kernel() {
  exec arroba-kernel "$@"
}

standalone() {
  arroba-kernel &
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
    exec "$@"
    ;;
  *)
    exec "$@"
    ;;
esac
