#!/bin/bash
# MP-08 / MP-09 / MP-10 / MP-11: coordinator-only planned restart after the account switch.
# Existing rounds retain their launch permissions; next Claude rounds use YOLO. Never relaunch work to change permissions.
set -euo pipefail
B3=${B3:-$HOME/.chariox/dev/browser-computer-use/resume-20260930/b3.sh}
S=${LANES_STATE:-/root/.chariox/dev/lanes-kernel}
R=${ADAPTERS:-/root/work/agent-dogfood/adapters}
C=${CLAUDE_ACCT:-/root/.chariox/dev/coordinator/claude-acct.sh}
printf -v COMMAND 'LANES_KERNEL_STATE=%q CLAUDE_ACCT=%q TMPDIR=%q timeout 600 node %q restart-yolo' "$S" "$C" "$S/tmp" "$R/tools/lanes/kernel/lanes-kernel.mjs"
if [ "$B3" = local ]; then
  bash -c "$COMMAND" || { echo 'FAILED: lanes kernel restart or carry-over check failed; inspect restart-rounds.json and loopd.log; do not repeat the restart blindly' >&2; exit 1; }
else
  "$B3" "$COMMAND" || { echo 'FAILED: lanes kernel restart or carry-over check failed; inspect restart-rounds.json and loopd.log; do not repeat the restart blindly' >&2; exit 1; }
fi
