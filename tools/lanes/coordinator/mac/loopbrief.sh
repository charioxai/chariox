#!/bin/bash
# loopbrief.sh <lane> <worktree> <claude|codex> < brief.md — the drop-in replacement for brief.sh once rounds run through Chariox.
# Copies the brief to agents/<lane>/BRIEF-<round>.md on b3, drops one queue entry for loopd and waits until loopd has picked it up.
# Env: MODEL (default claude-sonnet-5-5 / gpt-6.1-sol), EFFORT (high), HARD_STOP_MIN (240), ROUND (default: the next rN),
#      EXTRA_WORKTREES="/path /path" (more repositories the lane changes), PREAMBLE="/path /path" (shared rules loopd appends
#      to the brief; default: agents/common.md when it exists), B3 (the b3.sh helper), WAIT_SECONDS (120).
set -euo pipefail
L=${1:?lane} W=${2:?worktree} K=${3:?claude or codex}
case $K in
  claude) M=${MODEL:-claude-sonnet-5-5};;
  codex) M=${MODEL:-gpt-6.1-sol};;
  *) echo "kind must be claude or codex" >&2; exit 2;;
esac
E=${EFFORT:-high} H=${HARD_STOP_MIN:-240} R=${ROUND:-} X=${EXTRA_WORKTREES:-} P=${PREAMBLE:-} T=${WAIT_SECONDS:-120}
[[ $L =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,38}$ ]] || { echo "invalid lane name: $L" >&2; exit 2; }
[[ $W == /* && $W != *[[:space:]]* ]] || { echo "worktree must be an absolute path without spaces" >&2; exit 2; }
[[ $H =~ ^[0-9]+$ && $H -ge 1 && $H -le 1440 ]] || { echo "HARD_STOP_MIN must be 1..1440" >&2; exit 2; }
[[ -z $R || $R =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,38}$ ]] || { echo "invalid ROUND: $R" >&2; exit 2; }
B3=${B3:-$HOME/.chariox/dev/browser-computer-use/resume-20260930/b3.sh}
F=$(mktemp); trap 'rm -f "$F"' EXIT
cat > "$F"
[ -s "$F" ] || { echo "empty brief on stdin" >&2; exit 2; }

read -r -d '' BODY <<'REMOTE' || true
set -euo pipefail
A=${LOOP_AGENTS:-/root/.chariox/dev/browser-resume-20260930/agents}
D=$A/$L; Q=$A/_loop/queue
mkdir -p "$D" "$Q/done" "$Q/rejected"
if [ -z "$R" ]; then
  n=$(ls "$D" "$Q" "$Q/done" 2>/dev/null | sed -n "s/^BRIEF-r\([0-9][0-9]*\)\.md$/\1/p;s/^$L-r\([0-9][0-9]*\)\.json$/\1/p" | sort -n | tail -1)
  R=r$(( ${n:-0} + 1 ))
fi
[ ! -e "$D/BRIEF-$R.md" ] || { echo "round $R of $L already has a brief; pass ROUND=<new id>" >&2; exit 3; }
[ ! -e "$Q/$L-$R.json" ] && [ ! -e "$Q/done/$L-$R.json" ] || { echo "queue already has $L-$R; pass ROUND=<new id>" >&2; exit 3; }
cat > "$D/BRIEF-$R.md"
[ -n "$P" ] || { [ ! -f "$A/common.md" ] || P=$A/common.md; }
export L R W K M E H X P D
node -e '
const e = process.env;
const worktrees = [e.W, ...(e.X || "").split(/\s+/).filter(Boolean)];
const preamble = (e.P || "").split(/\s+/).filter(Boolean);
process.stdout.write(JSON.stringify({ lane: e.L, round: e.R, provider: e.K, model: e.M, effort: e.E, hardStopMinutes: Number(e.H), brief: `${e.D}/BRIEF-${e.R}.md`, ...(preamble.length && { preamble }), worktrees }, null, 2) + "\n");
' > "$Q/.$L-$R.json.tmp"
mv "$Q/.$L-$R.json.tmp" "$Q/$L-$R.json"
for ((i = 0; i < T; i += 2)); do
  if [ -e "$Q/rejected/$L-$R.json" ]; then echo "loopd rejected the entry:" >&2; cat "$Q/rejected/$L-$R.json.reason.txt" >&2; exit 3; fi
  if [ "$(node -e 'try { const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")); console.log(r.round === process.argv[2] && r.launchedAtMs ? "yes" : "no"); } catch { console.log("no"); }' "$D/LOOP_RUN.json" "$R")" = yes ]; then
    echo "queue file: $Q/$L-$R.json"; echo "run file:   $D/LOOP_RUN.json"; echo "picked up by loopd as $L-$R ($M $E, hard stop ${H} min)"; exit 0
  fi
  sleep 2
done
echo "loopd has not picked up $L-$R within ${T}s; is it running? node tools/lanes/kernel/lanes-kernel.mjs status (loopd.pid)" >&2
exit 4
REMOTE
VARS=$(printf 'L=%q K=%q W=%q M=%q E=%q H=%q R=%q X=%q P=%q T=%q LOOP_AGENTS=%q ' "$L" "$K" "$W" "$M" "$E" "$H" "$R" "$X" "$P" "$T" "${LOOP_AGENTS:-/root/.chariox/dev/browser-resume-20260930/agents}")
"$B3" "$VARS bash -c $(printf %q "$BODY")" < "$F"
