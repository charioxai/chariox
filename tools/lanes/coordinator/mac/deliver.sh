#!/bin/bash
# deliver.sh <lane> <review-file|->  — the review section for a worker running under loopd.
# 1. steers the running worker directly with the public CLI beneath the coordinator's grant (the running turn is steered; a
#    worker between turns gets the text as its next prompt) and prints the kernel's answer;
# 2. then appends the same section to agents/<lane>/REVIEW_INBOX.md for the record. loopd does not deliver it again: the steer
#    is recorded first, and loopd's inbox watch is only the fallback for a steer that failed (same text, same command id).
# A lane with no round under loopd (a classic lane) only gets the append. Exit 0 delivered; 4 steer failed, section appended;
# 2 usage.
# Env: B3 (the b3.sh helper), LANES_STATE, LOOP_AGENTS.
set -euo pipefail
L=${1:?lane} F=${2:?review file or -}
[[ $L =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,38}$ ]] || { echo "invalid lane name: $L" >&2; exit 2; }
B3=${B3:-$HOME/.chariox/dev/browser-computer-use/resume-20260930/b3.sh}
S=${LANES_STATE:-/root/.chariox/dev/lanes-kernel} A=${LOOP_AGENTS:-/root/.chariox/dev/browser-resume-20260930/agents}
T=$(mktemp); trap 'rm -f "$T"' EXIT
if [ "$F" = "-" ]; then cat > "$T"; else cat "$F" > "$T"; fi
[ -n "$(tr -d '[:space:]' < "$T")" ] || { echo "empty review text" >&2; exit 2; }
# The section is the text as given, with a heading when it has none, and always ends with a newline.
if ! grep -q '^## ' "$T"; then { printf '## %s coordinator\n' "$(date -u +%H:%MZ)"; cat "$T"; } > "$T.h" && mv "$T.h" "$T"; fi
[ -z "$(tail -c1 "$T")" ] || echo >> "$T"

read -r -d '' BODY <<'REMOTE' || true
set -uo pipefail
S=$(mktemp); cat > "$S"; trap 'rm -f "$S"' EXIT
R=$STATE/run/tools/lanes
export LANES_KERNEL_STATE=$STATE LOOPD_AGENTS=$AGENTS
INBOX=$AGENTS/$LANE/REVIEW_INBOX.md
mkdir -p "$AGENTS/$LANE"
steer=$(node "$R/kernel/lanes-kernel.mjs" exec --seat coordinator -- node "$R/coordinator/steer.mjs" "$LANE" "$S" 2>&1); code=$?
[ ! -s "$INBOX" ] || [ -z "$(tail -c1 "$INBOX")" ] || echo >> "$INBOX"
cat "$S" >> "$INBOX"
if [ $code -eq 0 ]; then echo "steered $LANE directly: $steer"; echo "appended to $INBOX for the record"; exit 0; fi
case $steer in
  *lane_not_running*) echo "$LANE has no round under loopd: appended to $INBOX only (classic lane)"; exit 0;;
  *) echo "direct steer of $LANE failed: $steer" >&2; echo "appended to $INBOX; loopd delivers it at its next wake" >&2; exit 4;;
esac
REMOTE
"$B3" "$(printf 'STATE=%q AGENTS=%q LANE=%q bash -c %q' "$S" "$A" "$L" "$BODY")" < "$T"
