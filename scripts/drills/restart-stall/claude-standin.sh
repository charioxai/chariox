#!/bin/sh
# Stand-in for the Claude Code CLI in stream-json mode, so a drill times only
# the kernel. With sessions=lost (run.sh writes the line below), like the real
# CLI on a machine without the session's files, it cannot resume a session and
# the kernel must rebuild the agent's context; with sessions=kept it resumes. A
# handoff-brief request gets a well-formed brief at once; any other turn
# answers at once. A drill probe's arrival (the last probe named in the
# message: a rebuilt handoff quotes earlier ones before the request) is logged (probes=<file>, written by
# run.sh) with its time: that is the agent's way back to work.
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
    *'drill-seed-'*)
      # Give the Room protected values through the kernel's own Vault tool.
      config=$(printf '%s' "$*" | sed -n 's/.*--mcp-config \([^ ]*\).*/\1/p')
      count=$(printf '%s' "$line" | sed -n 's/.*drill-seed-\([0-9]*\).*/\1/p')
      python3 "$(dirname "$0")/mcp_vault_seed.py" "$config" "${count:-25}" 2>> "$(dirname "$probes")/seed.log"
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"seeded"}]}}\n'
      printf '{"type":"result","subtype":"success","user_message_uuids":["%s"]}\n' "$uuid" ;;
    *'drill-stream-'*)
      # A long turn that streams output: the busy agent of the latency drill.
      i=0
      while [ $i -lt 150 ]; do
        printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"streamed progress %s: still working through the long task"}]}}\n' "$i"
        sleep 0.1
        i=$((i + 1))
      done
      printf '{"type":"result","subtype":"success","user_message_uuids":["%s"]}\n' "$uuid"
      marker=$(printf '%s' "$line" | grep -o 'drill-stream-[a-z0-9-]*' | tail -1)
      printf '%s\t%s\n' "$marker" "$(date +%s%3N)" >> "$(dirname "$probes")/endturn.tsv" ;;
    *)
      probe=$(printf '%s' "$line" | grep -o 'drill-probe-[a-z0-9]*-agent-[0-9a-f]*' | tail -1)
      [ -z "$probe" ] || printf '%s\t%s\n' "$probe" "$(date +%s%3N)" >> "$probes"
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Back after the restart."}]}}\n'
      printf '{"type":"result","subtype":"success","user_message_uuids":["%s"]}\n' "$uuid"
      marker=$(printf '%s' "$line" | grep -o 'drill-turn-[a-z0-9-]*' | tail -1)
      [ -z "$marker" ] || printf '%s\t%s\n' "$marker" "$(date +%s%3N)" >> "$(dirname "$probes")/endturn.tsv" ;;
  esac
done
