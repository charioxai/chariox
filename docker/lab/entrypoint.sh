#!/usr/bin/env sh
set -eu

mkdir -p "$HOME/.config/arroba" "$HOME/.local/share/arroba" "$HOME/.cache/arroba/runtime" "$HOME/workspace"

if [ -n "${ARROBA_MACHINE_ALIAS:-}" ]; then
  export ARROBA_MACHINE_ALIAS
elif [ -n "${HOSTNAME:-}" ]; then
  export ARROBA_MACHINE_ALIAS="$HOSTNAME"
fi

case "${1:-daemon}" in
  daemon)
    shift || true
    exec arroba-daemon "$@"
    ;;
  relay)
    shift || true
    exec arroba-relay "$@"
    ;;
  cli|arroba)
    shift || true
    exec arroba "$@"
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
