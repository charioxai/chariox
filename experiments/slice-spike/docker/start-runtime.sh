#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${ARROBA_SLICE_ROOT:-/opt/arroba-slice}"
LOGS="$ROOT/logs"
KERNEL_PORT="${ARROBA_SLICE_KERNEL_PORT:-43119}"
mkdir -p "$LOGS"

screen -S arroba-slice-relay -X quit >/dev/null 2>&1 || true
screen -S arroba-slice-kernel -X quit >/dev/null 2>&1 || true

screen -dmS arroba-slice-relay "$ROOT/bin/arroba-relay"
sleep 1
screen -dmS arroba-slice-kernel env ARROBA_KERNEL_PORT="$KERNEL_PORT" "$ROOT/bin/arroba-kernel"

screen -ls | sed -n '/arroba-slice-/p'

