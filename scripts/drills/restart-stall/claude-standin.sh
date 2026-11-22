#!/bin/sh
# Stand-in for the Claude Code CLI in stream-json mode, so a drill times only
# the kernel. With sessions=lost (run.sh writes the line below), like the real
# CLI on a machine without the session's files, it cannot resume a session and
# the kernel must rebuild the agent's context; with sessions=kept it resumes. A
# handoff-brief request gets a well-formed brief at once; any other turn shows
# its first output at once and then keeps working until the kernel stops it,
# so a later cold restart finds agents mid-turn again.
case "$*" in
  *'auth status'*) printf '{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max"}\n'; exit 0 ;;
  *--version*) printf '2.1.0 (Claude Code)\n'; exit 0 ;;
esac
case "$sessions:$*" in
  lost:*--resume*) echo 'No conversation found with session ID' >&2; exit 1 ;;
esac
# Each turn starts with the CLI's init report, under the resumed session or a new one.
session=$(printf '%s' "$*" | sed -n 's/.*--resume \([^ ]*\).*/\1/p')
[ -n "$session" ] || session=$(cat /proc/sys/kernel/random/uuid)
brief='## Goal\nContinue the work\n## Constraints & Preferences\nNone\n## Progress\n### Done\nRecorded\n### In Progress\nNone\n### Blocked\nNone\n## Key Decisions\nNone\n## Next Steps\nContinue\n## Critical Context\nNone'
while IFS= read -r line; do
  case "$line" in
    *'"type":"user"'*) ;;
    *) continue ;;
  esac
  uuid=$(printf '%s' "$line" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
  printf '{"type":"system","subtype":"init","session_id":"%s"}\n' "$session"
  case "$line" in
    *'Update the handoff brief'*)
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"%s"}]}}\n' "$brief"
      printf '{"type":"result","subtype":"success","user_message_uuids":["%s"]}\n' "$uuid" ;;
    *)
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Back after the restart."}]}}\n' ;;
  esac
done
