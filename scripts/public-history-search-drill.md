# MP-08 / MP-10 / MP-11 — A09 real history search replay

This is a user-driven replay for PR9. It requires the exact built kernel/TUI and
an authorized Chariox-linked official provider profile. Use the profile authorized for the current round. Round 3 permits linking
the existing `/root/.codex-agents` login through the product command. Do not substitute a stub
provider, synthetic page or internal IPC call for a live acceptance step. Record
source/binary hashes, exit codes and screenshots per step outside the checkout.

## MP-08 / MP-10 / MP-11: launch

Coordinator supplies `AM9_KERNEL`, `AM9_TUI`, `AM9_CHECKOUT`, `AM9_PROVIDER`,
`AM9_LINKED_PROFILE` (a product account ID in this kernel), and unused
`AM9_KERNEL_PORT` / `AM9_MCP_PORT`. First link the authorized native provider
profile through the TUI's `/provider accounts link` flow, then refresh its status;
never read or copy its credential files. Use a disposable lane-owned state root:

```bash
mkdir -p "$HOME/.chariox/dev"
export CHARIOX_HOME="$(mktemp -d "$HOME/.chariox/dev/am9-live.XXXXXX")"
export CHARIOX_ROOM_AGENT_TOOLS=1 CHARIOX_KERNEL_HOST=127.0.0.1
export CHARIOX_KERNEL_PORT="$AM9_KERNEL_PORT" CHARIOX_MCP_PORT="$AM9_MCP_PORT"
export CHARIOX_DAEMON_SOCKET="$CHARIOX_HOME/daemon.sock"
"$AM9_KERNEL"
```

In a second terminal with the same environment:

```bash
"$AM9_TUI" --kernel-url "ws://127.0.0.1:$AM9_KERNEL_PORT/kernel" \
  --create-session --workspace "$AM9_CHECKOUT" --worktree "$AM9_CHECKOUT" \
  --provider "$AM9_PROVIDER" --account-profile "$AM9_LINKED_PROFILE"
```

## MP-08 / MP-10 / MP-11: replay A09 S01–04

1. On base PR1, ask the real agent to inspect the repository and use
   `chariox.history.search` to retrieve a peer's earlier review. Capture the actual
   missing-tool failure. If account readiness fails first, report BLOCKED at
   account readiness; it is not a history-search RED result.
2. On PR9, ask the root agent to recruit actual peer reviewers for two relevant
   repository modules. Let them inspect source and exchange useful findings through
   normal room messages. Ask a reviewer to find the earlier task and another
   peer's findings using `chariox.history.search`, then read a returned `event_ref`.
   Capture actual tool calls, bounded hits and the useful final answer in the TUI.
3. Ask for a one-hit page of a real repeated task term, follow the returned cursor
   unchanged, and verify no duplicates or skipped retained public findings. During
   a rebuild retry the first query until this room's `coverage.rebuilding=false`.
   Capture exclusion/truncation/retention coverage without interpreting it as full
   provider archival coverage.
4. Create a separate real room through the TUI and run a distinct useful review.
   Have the first room's agent request that room explicitly and try its event
   reference: search is denied without live same-owner sudo; a foreign reference
   and a guessed reference have the same unavailable result. An ordinary room
   query must contain no other-room hits, counts, snippets or rebuild progress.
5. Exercise the real owner-approved protected Vault/login/hand-off workflow in the
   integrated candidate, including its official provider echo/encoded-echo cases.
   Ask the agent to search/read around the related public task. Protected input,
   private reasoning, auth tool records and attachment bodies must be absent from
   the public index and detail output. Do not put actual secret values into drill
   prompts, logs or evidence; inspect via product protection status and safe public
   task references. Capture invalidated references/cursors and coverage after
   protected provenance changes, deletion and rotation.
6. Invoke an owner-approved real private MCP service whose normalized tool name
   is generic (for example `read`) and whose server is private (for example
   `vault`). Search/read its safe public task references from a peer. The whole
   private tool record must be unavailable, including values not previously
   registered in the Room. Never copy a real credential into the prompt/evidence.
   If the real service/account is absent, report BLOCKED; the normalized-shape
   regression is supplementary.
7. Reconnect the real TUI, then restart only the owned kernel/provider at the
   appendix boundaries. Finish actual review work and reconcile retained public
   history after restart. Run A09 S04 retained/rebuild/legacy-provenance, S03
   protected invalidation and S02 forbidden global/foreign scope cases. Report any unavailable prerequisite as
   an exact coordinator blocker, never a fixture pass.

## MP-08 / MP-10 / MP-11: round 2 retained-history regressions

Use the real official provider and the authorized worker/relay resources. These
steps require authorized live worker/relay resources. Existing local Codex
readiness does not establish worker or hosted readiness.
Source regressions do not substitute for them.

- Search/read useful peer review findings before leased projection initialization,
  then continue actual worker execution and search/read the same references after
  projection acknowledgements and tool-state compaction. The retained sanitized
  findings must remain available.
- Run concurrent useful peer reviews while paging a repeated task term. If a
  delayed lower-sequence append enters the room's retained window, the existing
  cursor must report stale. Restart the query and page all retained hits once.
  Later increasing-sequence appends must preserve the bounded cursor.
- After a newer useful review emits more than 200 public events, request the
  previous retained peer turn through `chariox.history.turn` using both
  `turns_back: 1` and its returned `turn_id` as `turn_ref`. Bound the returned
  events with `limit`; the newer turn must not hide the previous one.

## MP-10 / MP-11: full acceptance and cleanup

Execute every A09 G01–G18 / S01–S04 axis cell using the frozen appendix: all three
real official providers, four real clients, three placements and ordinary/managed
Path-1. Hosted enrollment, integrated prerequisites, real public sites/accounts,
DPR1/2, measured hosted wss network, latency/frame-rate thresholds and multi-hour
stability require the coordinator's actual validation resources. The local replay
above is only one leg. Source tests are supplementary.

Stop only processes this run owns (PID > 1 and original process identity), verify
owned descendants exited, inventory the exact disposable state path and remove
its generated runtime identities/materialized profiles. Preserve native linked
profiles, durable key stores, shared reviewer state and all other lanes' resources.

MP-08 / MP-10 / MP-11: [round 3 live results](../docs/PUBLIC_HISTORY_SEARCH_LIVE_VALIDATION.md)
record local passes, newly reproduced failures and exact protected-service blockers.
