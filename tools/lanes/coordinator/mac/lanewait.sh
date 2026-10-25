#!/bin/bash
# lanewait.sh [MINUTES] [--json]  |  lanewait.sh --replay N
# Blocks until a loopd worker ends a turn, finishes, fails, raises a decision loopd does not answer, or goes silent
# (LONG-ROUND / SILENT), then prints one line per event: time, lane, round, kind, summary. It replaces watch.sh's lane
# polling; GitHub review polling stays in watch.sh. Exit 0: lines printed. Exit 3: nothing within MINUTES (default 240).
# It runs one long wait on b3 beneath the coordinator's own access grant (its own inbox: loopd keeps its events), over the
# one ssh connection the B3 helper holds. --replay N lists the last N lines again (lines lost on a dropped connection).
# Env: B3 (the b3.sh helper), LANES_STATE (default /root/.chariox/dev/lanes-kernel), LOOP_AGENTS.
set -euo pipefail
B3=${B3:-$HOME/.chariox/dev/browser-computer-use/resume-20260930/b3.sh}
S=${LANES_STATE:-/root/.chariox/dev/lanes-kernel} A=${LOOP_AGENTS:-/root/.chariox/dev/browser-resume-20260930/agents}
case ${1:-} in
  --replay) [[ ${2:-} =~ ^[0-9]+$ ]] || { echo "usage: lanewait.sh --replay N" >&2; exit 2; }; ARGS="--replay $2"; shift 2;;
  *) M=240; [[ ${1:-} =~ ^[0-9]+$ ]] && { M=$1; shift; }
     [[ $M -ge 1 && $M -le 1440 ]] || { echo "MINUTES must be 1..1440" >&2; exit 2; }
     ARGS="--minutes $M";;
esac
if [ "${1:-}" = "--json" ]; then ARGS="$ARGS --json"; shift; fi
[ $# -eq 0 ] || { echo "usage: lanewait.sh [MINUTES] [--json] | --replay N" >&2; exit 2; }
set +e
"$B3" "$(printf 'LANES_KERNEL_STATE=%q LOOPD_AGENTS=%q node %q/tools/lanes/kernel/lanes-kernel.mjs exec --seat coordinator -- node %q/tools/lanes/coordinator/lanewait.mjs %s' "$S" "$A" "$S/run" "$S/run" "$ARGS")"
code=$?
[ $code -ne 3 ] || echo "no worker events within the wait" >&2
exit $code
