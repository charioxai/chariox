#!/usr/bin/env bash
set -euo pipefail

mkdir -p "$ARROBA_CONFIG_DIR" "$ARROBA_DATA_DIR" "$ARROBA_RUNTIME_DIR" "$ARROBA_SESSION_HISTORY_DIR" /workspace

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
