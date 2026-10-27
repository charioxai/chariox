#!/usr/bin/env sh
set -eu

mkdir -p "$HOME/.config/chariox" "$HOME/.local/share/chariox" "$HOME/.cache/chariox/runtime" "$HOME/workspace"

if [ -n "${CHARIOX_MACHINE_ALIAS:-}" ]; then
  export CHARIOX_MACHINE_ALIAS
elif [ -n "${HOSTNAME:-}" ]; then
  export CHARIOX_MACHINE_ALIAS="$HOSTNAME"
fi

case "${1:-daemon}" in
  daemon)
    shift || true
    exec chariox-kernel "$@"
    ;;
  relay)
    shift || true
    exec chariox-relay "$@"
    ;;
  cli|chariox)
    shift || true
    exec chariox "$@"
    ;;
  shell|zsh)
    shift || true
    exec zsh "$@"
    ;;
  bash)
    shift || true
    exec bash "$@"
    ;;
  *)
    exec "$@"
    ;;
esac
