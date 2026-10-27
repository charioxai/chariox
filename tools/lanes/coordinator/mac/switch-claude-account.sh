#!/bin/bash
# switch-claude-account.sh chariox|old — moves the lanes to the other Claude account without stopping any worker.
# 1. probes the target token on b3 (HTTP 200 and limit status allowed or allowed_warning; only the organization id suffix is printed) and refuses
#    when it is not usable yet, for example the chariox account before its weekly reset (2026-10-25T16:00Z);
# 2. re-stores the running lanes kernel's lanes-claude credential in place (lanes-kernel.mjs set-claude-credential: no kernel
#    restart, no worker stopped). A Claude process takes its credential when it is launched, so the workers already running move
#    when their process is next relaunched (a round's relaunch, a restart, a resume) and every new launch uses the new account;
# 3. then copies the token to oauth-token, so classic launches use it too.
# The kernel comes first on purpose: if it refuses, nothing has changed. Exit 0 switched; 3 target not usable; 2 usage.
# Env: B3 (the b3.sh helper, or "local" when this runs on b3), LANES_STATE, LOOP_AGENTS, CLAUDE_TOKENS, ADAPTERS.
set -euo pipefail
case ${1:-} in
  chariox) F=oauth-token.chariox;;
  old) F=oauth-token.old-account;;
  *) echo "usage: switch-claude-account.sh chariox|old" >&2; exit 2;;
esac
PROBE_ONLY=0
if [ "${2:-}" = --probe ] && [ $# -eq 2 ]; then PROBE_ONLY=1; elif [ $# -ne 1 ]; then { echo "usage: switch-claude-account.sh chariox|old" >&2; exit 2; }
fi
B3=${B3:-$HOME/.chariox/dev/browser-computer-use/resume-20260930/b3.sh}
S=${LANES_STATE:-/root/.chariox/dev/lanes-kernel} A=${LOOP_AGENTS:-/root/.chariox/dev/browser-resume-20260930/agents}
D=${CLAUDE_TOKENS:-/root/.claude-lanes} R=${ADAPTERS:-/root/work/agent-dogfood/adapters}

read -r -d '' BODY <<'REMOTE' || true
set -uo pipefail
K=$ADAPTERS/tools/lanes/kernel
[ -f "$K/lanes-kernel.mjs" ] || { echo "no adapters checkout at $ADAPTERS" >&2; exit 4; }
[ -f "$TOKENS/$FILE" ] || { echo "no $FILE under $TOKENS" >&2; exit 4; }
probe=$(node "$K/claude-account.mjs" probe "$TOKENS/$FILE" 2>&1); code=$?
echo "probe: $probe"
[ $code -eq 0 ] || { echo "refused: the $FILE account is not usable yet; nothing changed" >&2; exit 3; }
[ "$PROBE_ONLY" = 0 ] || exit 0
set=$(LANES_KERNEL_STATE=$STATE node "$K/lanes-kernel.mjs" set-claude-credential --from "$TOKENS/$FILE" 2>&1); code=$?
echo "kernel: $set"
[ $code -eq 0 ] || { echo "the lanes kernel refused the credential; oauth-token left as it was" >&2; exit 5; }
if [ -d "$TOKENS/.oauth-token.new" ] || ! cp -p "$TOKENS/$FILE" "$TOKENS/.oauth-token.new" || [ -d "$TOKENS/oauth-token" ] || ! node -e 'require("node:fs").renameSync(process.argv[1], process.argv[2])' "$TOKENS/.oauth-token.new" "$TOKENS/oauth-token"; then
  echo "FAILED: kernel credential has already changed; classic token update failed; repair it before classic launches" >&2; exit 6
fi
if ! touch "$AGENTS/CLAUDE_SWITCHED"; then
  echo "FAILED: kernel credential has already changed and classic token was updated; switch marker failed; repair the marker" >&2; exit 7
fi
echo "oauth-token now holds the $FILE account; running workers keep their Claude process until it is relaunched"
REMOTE
if [ "$B3" = local ]; then
  STATE=$S AGENTS=$A TOKENS=$D ADAPTERS=$R FILE=$F PROBE_ONLY=$PROBE_ONLY bash -c "$BODY"
else
  "$B3" "$(printf 'STATE=%q AGENTS=%q TOKENS=%q ADAPTERS=%q FILE=%q PROBE_ONLY=%q bash -c %q' "$S" "$A" "$D" "$R" "$F" "$PROBE_ONLY" "$BODY")"
fi
