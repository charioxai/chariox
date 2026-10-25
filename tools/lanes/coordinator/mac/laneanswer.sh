#!/bin/bash
# laneanswer.sh <lane> <decision> <choice> — answers a decision lanewait listed (kind "decision"), with one of the choices it printed.
# Critical, passkey and credential decisions are the owner's: the kernel refuses them for the coordinator's grant and says so.
# Env: B3 (the b3.sh helper), LANES_STATE, LOOP_AGENTS.
set -euo pipefail
L=${1:?lane} D=${2:?decision id} C=${3:?choice id}
[[ $L =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,38}$ ]] || { echo "invalid lane name: $L" >&2; exit 2; }
[[ $D =~ ^[A-Za-z0-9][A-Za-z0-9_.:-]{0,200}$ && $C =~ ^[A-Za-z0-9][A-Za-z0-9_.:-]{0,100}$ ]] || { echo "invalid decision or choice id" >&2; exit 2; }
B3=${B3:-$HOME/.chariox/dev/browser-computer-use/resume-20260930/b3.sh}
S=${LANES_STATE:-/root/.chariox/dev/lanes-kernel} A=${LOOP_AGENTS:-/root/.chariox/dev/browser-resume-20260930/agents}
"$B3" "$(printf 'LANES_KERNEL_STATE=%q LOOPD_AGENTS=%q node %q/tools/lanes/kernel/lanes-kernel.mjs exec --seat coordinator -- node %q/tools/lanes/coordinator/answer.mjs %q %q %q' "$S" "$A" "$S/run" "$S/run" "$L" "$D" "$C")"
