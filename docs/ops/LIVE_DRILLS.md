# Live Drill Gate

This is the post-M4.5 gate before starting new feature work. The M4.5 ownership points are closed; confidence now comes from live behavior across local providers, workflows, and relay-backed remote machines.

Script-level policy lives in `apps/cli/scripts/README.md`: live drills default to older Codex-capable models such as `gpt-5.2`, low effort, and OpenCode runs use OpenAI Codex-family models instead of OpenCode provider-default or `zen` models.

Terminology:

- **Freeform multi-agent mode** means a normal multi-agent session with no workflow scheduler. Agents can be focused, prompted, and observed independently, but no workflow graph owns turn progression.
- **Workflow mode** means a daemon-owned workflow graph schedules node turns, validates outputs, and advances runs.
- **Remote mode** means one or more CLIs or agents connect through relay to a home daemon or to a remote machine accepting leases.

## Required Drill Matrix

1. Freeform local multi-agent:
   - Start one session.
   - Attach at least one local CLI.
   - Spawn at least one OpenCode-backed agent and one Codex-backed agent.
   - Submit targeted prompts to both agents.
   - Verify each agent completes without blocking the other, focus/state projections remain correct, and provider output is visible from the attached CLI.

2. Local workflow drills:
   - Run the existing workflow drill catalog:
     - `simple-chain`
     - `validated-increment-chain`
     - `console-increment-chain`
     - `final-run-output-chain`
     - `cyclic-final-run-output-chain`
     - `cyclic-budgeted-final-run-output-chain`
     - `cyclic-final-run-with-intermediate-output-chain`
     - `conditional-branch-subset`
     - `immediate-release-downstream`
   - Verify run status, node summaries, final output, intermediate output, console entries, provider cleanup, and session projection refresh.

3. Remote freeform multi-agent through relay:
   - Start relay, home daemon, and worker daemon.
   - Attach one local CLI directly and one remote CLI through relay.
   - Spawn local and remote agents through the home session.
   - Submit targeted prompts from both CLIs, including prompts to worker-machine agents.
   - Restart relay during the session and verify transport close/resume plus continued prompt completion.

4. Remote workflow through relay:
   - Start relay, home daemon, and worker daemon.
   - Run the workflow drill catalog with agents leased on the remote worker machine.
   - Verify workflow completion, runtime-tool handling, relay recovery, and final projection state from both local and relayed clients.

## Existing Automation

- Local freeform multi-agent: `node apps/cli/scripts/live-freeform-multi-agent-drill.mjs --providers opencode,codex`
- Local managed I/O: `node apps/cli/scripts/live-managed-io-drill.mjs --providers opencode,codex` or `pnpm --filter @arroba/cli run managed-io:drill`
- Local workflow: `node apps/cli/scripts/live-workflow-runtime-drill.mjs --spawn-daemon --scenario <scenario> --providers opencode,codex`
  - Add `--no-early-pass` when validating the full `immediate-release-downstream` completion path instead of only the immediate release point.
- Remote freeform multi-agent: `node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs --providers opencode,codex`
- Remote managed I/O smoke: `node apps/cli/scripts/live-remote-managed-io-drill.mjs --providers opencode,codex`
- Remote managed I/O full: `node apps/cli/scripts/live-remote-managed-io-drill.mjs --providers opencode --full` or `pnpm --filter @arroba/cli run managed-io:remote-drill`
- Remote workflow: `node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario <scenario> --providers opencode,codex`
- Lower-level relay runtime: `node apps/cli/scripts/live-relay-runtime-drill.mjs`
- Lower-level remote machine runtime: `node apps/cli/scripts/live-remote-machine-runtime-drill.mjs`

## Exit Criteria

- Every required drill either passes or has a filed blocker with repro command, observed failure, and owner.
- `cargo check --manifest-path apps/daemon/Cargo.toml` remains clean.
- Daemon lib, runtime integration, websocket integration, relay-client, and bin tests remain green after any drill fixes.
- Docs reflect the final drill results before new tasks begin.

## Current Results

- Freeform local multi-agent: **pass** with `opencode,codex` after the router bootstrap lock fix.
- Local managed I/O: **pass** with `opencode,codex`; agents read `seed.txt`, create provider-specific output files through `arroba.write_artifact`, edit/apply-patch/move/delete through managed tools, fail if direct/native write attempts create forbidden files, serialize same-area agent collisions to one winning write, rebase stale non-overlapping external changes, and reject stale overlapping external changes. The drill owns and tears down its daemon, session, isolated workspace, session history, and transient CLI module cache.
- Remote managed I/O: **full pass with `opencode`** and **smoke pass with `codex`** after leased runs require managed I/O. OpenCode covers managed read/write/edit/apply-patch/move/delete, direct-write blocking, same-area collision serialization, stale non-overlap external-change rebase, and stale overlap external-change rejection. Codex full remote remains blocked in the overlap phase, where the drill times out waiting for both overlapping `edit_artifact` results.
- Local workflow catalog: **pass** with spawned local daemon and `opencode,codex`.
  - `simple-chain`
  - `validated-increment-chain`
  - `console-increment-chain`
  - `final-run-output-chain`
  - `cyclic-final-run-output-chain`
  - `cyclic-budgeted-final-run-output-chain`
  - `cyclic-final-run-with-intermediate-output-chain`
  - `conditional-branch-subset`: router node emits structured `workflow_handoffs`, only the selected even-value branch runs, and the excluded branch is not invoked.
  - `immediate-release-downstream`: producer submits validated intermediate output, downstream starts before producer completion, consumes `{"value":1842}`, and submits final workflow output `{"value":1843}`. Full completion is verified with both provider orders using `--no-early-pass`.
- Remote freeform relay: **pass** with `opencode,codex`, direct local client plus relayed client, local sidecar agents plus worker-machine leased agents, relay restart, transport close/resume, and post-reconnect prompt completion.
- Remote workflow relay: **pass** with `opencode,codex`, relay, home daemon, worker daemon, remote worker-machine leases, forwarded workflow runtime tools, cyclic workflow progression, intermediate output, final workflow output, and clean final projections.
- Workflow prompt-injection invariant: **pass**. Workflow prompt construction is scheduler-owned and shared by local dispatch, remote dispatch, retry/replay paths, and tests; the renderer injects turn index, last-turn guidance only on the last allowed node turn, final-output tool guidance, runtime-tool instructions, handoff payloads, edge contracts, and control mailbox content.

Point-2 fixes proven by the local workflow catalog:

- Workflow endpoint invocation now enqueues the entry dispatches returned by owned invoke admission instead of dropping them.
- Owned workflow prompt rendering now includes workflow prompt, node instructions, outgoing edge contracts, ack/validation/tool instructions, and the final fenced JSON contract for entry and downstream turns.
- Terminal pumping drains all starting/running provider runs in the session, not only the focused active provider.
- Structured provider lifecycle handling ignores stale completion/idle events and requires a submitted prompt before treating idle as completion.
- Workflow console writes no longer deadlock by taking a session-store read while constructing arguments for a write-locked mutation.
- Resolved missing-output/output-validation failures are cleared when a later successful turn for the same workflow node produces output or submits final output.
- Node max-turn budget accounting counts completed turns with completion payloads, so recovered missing-output attempts do not consume the production success budget.
- Intermediate-output runtime tools now enqueue the downstream prompt dispatches they prepare; the previous path created downstream node runs but dropped the async provider submission work.
- Final workflow-output submissions no longer generate downstream handoff validation failures while completion is still committing pending turn outputs.
- The budgeted cyclic final-output scenario now checks the exact final output payload, so a stale pre-ack provider output cannot accidentally satisfy a later final workflow turn.
- Conditional branching drill: **pass**. A router node with two outgoing edges emitted a structured `workflow_handoffs` payload selecting only the even branch; the selected branch completed with `{"bucket":"even","values":[2]}` and the excluded branch was not invoked.
- Immediate-release drill: **pass for release semantics**. A valid intermediate output `{"value":1842}` created and prompted the downstream node while the producer node was still running, proving asynchronous handoff release before producer turn completion. Provider settlement after the intermediate tool remains a separate follow-up hardening item.

Point-3 evidence:

```
node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs --providers opencode,codex --model gpt-5.2 --timeout-ms 240000 --poll-ms 1000
```

Observed result: both providers were visible on the worker machine, both providers had local sidecar agents and remote leased worker agents, both local and relayed clients observed four assistant completions, the relayed client observed `transport_closed` and `transport_resumed`, and both remote worker agents completed prompts after relay restart.

Point-4 evidence:

```
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario validated-increment-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario simple-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario console-increment-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario final-run-output-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario cyclic-final-run-output-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario cyclic-budgeted-final-run-output-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario cyclic-final-run-with-intermediate-output-chain --providers opencode,codex --model gpt-5.2 --poll-limit 700 --poll-interval-ms 250
```

Observed result: the remote workflow catalog completed through the relay with worker-machine leased agents. The closing cyclic intermediate-output run completed with validated intermediate output and final workflow output from `workflow-node-run-2`, final output `{"value":1843}`, no failure events, and status `Completed`.
