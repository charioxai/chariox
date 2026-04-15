# Live Drill Gate

This is the post-M4.5 gate before starting new feature work. The M4.5 ownership points are closed; confidence now comes from live behavior across local providers, workflows, and relay-backed remote machines.

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
- Local managed I/O: `node apps/cli/scripts/live-managed-io-drill.mjs --providers opencode,codex`
- Local workflow: `node apps/cli/scripts/live-workflow-runtime-drill.mjs --spawn-daemon --scenario <scenario> --providers opencode,codex`
- Remote freeform multi-agent: `node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs --providers opencode,codex`
- Remote workflow: `node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario <scenario> --provider codex` and the same command with `--provider opencode`
- Lower-level relay runtime: `node apps/cli/scripts/live-relay-runtime-drill.mjs`
- Lower-level remote machine runtime: `node apps/cli/scripts/live-remote-machine-runtime-drill.mjs`

## Exit Criteria

- Every required drill either passes or has a filed blocker with repro command, observed failure, and owner.
- `cargo check --manifest-path apps/daemon/Cargo.toml` remains clean.
- Daemon lib, runtime integration, websocket integration, relay-client, and bin tests remain green after any drill fixes.
- Docs reflect the final drill results before new tasks begin.

## Current Results

- Freeform local multi-agent: **pass** with `opencode,codex` after the router bootstrap lock fix.
- Local managed I/O: **pass** with `opencode,codex`; agents read `seed.txt`, create provider-specific output files through `arroba.write_artifact`, and fail the drill if direct/native write attempts create forbidden files.
- Local workflow catalog: **pass** with spawned local daemon and `opencode,codex`.
  - `simple-chain`
  - `validated-increment-chain`
  - `console-increment-chain`
  - `final-run-output-chain`
  - `cyclic-final-run-output-chain`
  - `cyclic-budgeted-final-run-output-chain`
  - `cyclic-final-run-with-intermediate-output-chain`
- Remote freeform relay: **pass** with `opencode,codex`, direct local client plus relayed client, local sidecar agents plus worker-machine leased agents, relay restart, transport close/resume, and post-reconnect prompt completion.
- Remote workflow relay: **not started in this gate**.

Point-2 fixes proven by the local workflow catalog:

- Workflow endpoint invocation now enqueues the entry dispatches returned by owned invoke admission instead of dropping them.
- Owned workflow prompt rendering now includes workflow prompt, node instructions, outgoing edge contracts, ack/validation/tool instructions, and the final fenced JSON contract for entry and downstream turns.
- Terminal pumping drains all starting/running provider runs in the session, not only the focused active provider.
- Structured provider lifecycle handling ignores stale completion/idle events and requires a submitted prompt before treating idle as completion.
- Workflow console writes no longer deadlock by taking a session-store read while constructing arguments for a write-locked mutation.
- Resolved missing-output/output-validation failures are cleared when a later successful turn for the same workflow node produces output or submits final output.
- Node max-turn budget accounting counts completed turns with completion payloads, so recovered missing-output attempts do not consume the production success budget.
- Final workflow-output submissions no longer generate downstream handoff validation failures while completion is still committing pending turn outputs.

Point-3 evidence:

```
node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs --providers opencode,codex --model gpt-5.4 --timeout-ms 240000 --poll-ms 1000
```

Observed result: both providers were visible on the worker machine, both providers had local sidecar agents and remote leased worker agents, both local and relayed clients observed four assistant completions, the relayed client observed `transport_closed` and `transport_resumed`, and both remote worker agents completed prompts after relay restart.
