# Chariox Server Provider Thread Transfer Drills Plan

## Goal

Validate whether an autonomous Chariox agent can be made compliant with an
Chariox Server slice requirement while preserving the same provider-native
thread.

This plan answers one open design question:

```text
Can a running agent move from local execution into a standard Chariox slice and
continue as the same provider thread?
```

Credentials for providers available on the main machine are assumed to be
available in the slice. These drills therefore focus on provider thread
portability, provider-local state, duplicate-run prevention, terminal fanout,
and rollback.

## Definitions

Provider thread means the provider-native conversation identity:

- Codex: Codex thread id
- OpenCode: OpenCode session id
- Claude Code: Claude session id

Same agent means the same Chariox agent record continuing through the same
provider-native thread. The provider process and Chariox provider run id may
change. The provider thread id must remain the same.

Transfer means:

```text
local provider run
  -> capture provider resume state
  -> stop or park local provider run
  -> prepare slice
  -> launch provider in slice with same resume state
  -> verify same provider thread id
  -> continue turn handling through client kernel and terminals
```

Failure means the kernel cannot prove same-thread continuity. On failure, the
kernel must not silently start a blank replacement thread.

## Providers Under Test

- Codex
- OpenCode
- Claude Code

## Existing Surfaces To Inspect

The drills should be grounded in existing runtime surfaces:

- `ProviderResumeState` and provider session ids in provider launch contracts
- provider reload policy for MCP and launch-time config changes
- remote leased native provider launch with optional provider session id
- current `move_agent_to_remote` limitation that refuses agents with provider
  runs
- slice saved state and slice auth import behavior
- terminal fanout and provider output projection

## Success Criteria

For each provider, a successful transfer must prove:

- the pre-transfer provider thread id is captured
- the original provider run is stopped, parked, or otherwise prevented from
  accepting more turns
- the slice provider run starts with the same provider thread id
- a post-transfer prompt observes prior conversation state
- Chariox terminals attached to the session continue to receive coherent output
- server-directed workflow endpoint invocation still reaches the client kernel
- no duplicate live provider run exists for the same Chariox agent and provider
  thread
- rollback restores the exact previous local configuration when transfer fails

## Failure Criteria

The design should be considered unsupported for a provider or scenario if:

- the provider cannot resume by thread/session id inside the slice
- the provider requires local state that cannot be identified or transferred
  safely
- resume creates a forked or duplicate conversation
- the provider reports a different thread id after transfer
- terminal fanout loses provider output or prompt lifecycle state
- rollback cannot restore the exact previous configuration

If a provider fails, V1 should require autonomous agents that need strict
slice-backed server admission to start inside the standard Chariox slice instead
of moving there later.

## Evidence To Capture

Each drill run should save:

- provider name and version
- local provider thread id before transfer
- provider run id before transfer
- slice provider thread id after transfer
- provider run id after transfer
- MCP/tool/permission configuration before and after transfer
- terminal transcript before and after transfer
- workflow invocation id, if the post-transfer prompt comes from a workflow
  endpoint
- logs proving the old run was stopped or parked
- rollback logs for negative drills

Artifacts should be saved under `./.artifacts/provider-thread-transfer/`.

## Current Executable Coverage

`apps/cli/scripts/live-provider-thread-transfer-drill.mjs` implements the
first executable drill in this plan:

```bash
pnpm --filter @chariox/cli run provider-thread-transfer:drill
```

The implemented drill modes are:

- `local-reload`, which validates Drill 1. It starts an isolated kernel,
  creates a session and agent, sends a marker prompt, captures the
  provider-native thread id, grants a deterministic local MCP to force provider
  reload, and sends a second marker prompt through the reloaded run. This is
  not slice migration; it is only the local reload baseline.
- `worker-resume`, which is a Drill 3 precursor. It starts a local relay, home
  kernel, and same-host worker kernel; creates a local provider thread; ends
  the local provider run; spawns a remote-backed agent; launches the worker
  provider through the existing remote-native launch path with the captured
  provider session id; and verifies same-thread recall. This proves provider
  thread portability through a worker launch, but it does not yet prove moving
  the same Chariox agent record.
- `slice-restart`, which validates a real home-managed local Docker slice
  lifecycle. It creates a slice, imports provider auth, spawns a slice-backed
  agent, launches the provider through the leased worker path, sends a marker
  prompt, saves the slice state with `restart_agents`, and verifies that the
  same Chariox agent record continues with the same provider-native thread after
  slice restart.
- `live-migrate-to-slice`, which validates the hard Drill 4 path. It starts
  the provider thread on the main machine, captures the provider-native thread
  id, creates and starts a local Docker slice, copies provider-local state into
  the slice, moves the same Chariox agent record to the slice worker, lets the
  move operation terminate the idle local provider run, launches provider
  execution in the slice with the captured resume state, and verifies
  same-thread recall through the client kernel path.

`worker-resume` supports two worker-state modes:

- `--worker-state shared`, the default, shares the normal provider home,
  credential, data, and cache directories with the worker kernel.
- `--worker-state isolated` gives the worker an isolated provider home, data,
  state, and cache root, then copies only temporary provider auth material into
  a non-artifact temp directory. This is not a real slice, but it distinguishes
  provider thread ids that are resumable with credentials only from provider
  thread ids that require provider-local session state transfer.

Validated evidence on June 13, 2026:

- OpenCode passed in
  `.artifacts/provider-thread-transfer/1781384413326-63531/matrix.json`
  - provider run changed from `provider-run-1` to `provider-run-2`
  - provider thread id stayed
    `ses_13d36f6b7ffeaHZKaVbA1yuv2t`
  - reloaded run included `thread_transfer_probe_opencode_63531`
  - post-reload second-turn marker was observed
- Codex passed in
  `.artifacts/provider-thread-transfer/1781384905340-94452/matrix.json`
  - provider run changed from `provider-run-1` to `provider-run-2`
  - provider thread id stayed
    `019ec2d0-86a4-7d12-bb61-1e1f46eba8ba`
  - reloaded run included `thread_transfer_probe_codex_94452`
  - post-reload second-turn marker was observed

Same-host worker resume evidence on June 13, 2026:

- OpenCode passed with shared worker state in
  `.artifacts/provider-thread-transfer/1781385437161-30149/matrix.json`
  - local provider run ended before remote launch
  - provider thread id stayed
    `ses_13d274232ffec5B9kAwaIWSNhG`
  - worker remote-native launch reported the same thread id
  - worker recall marker was observed
- Codex passed with shared worker state in
  `.artifacts/provider-thread-transfer/1781385484757-34890/matrix.json`
  - local provider run ended before remote launch
  - provider thread id stayed
    `019ec2d9-7cb7-73b3-a8ef-d4f10837d73e`
  - worker remote-native launch reported the same thread id
  - worker recall marker was observed

Same-host worker resume with isolated worker state on June 13, 2026:

- OpenCode failed in
  `.artifacts/provider-thread-transfer/1781385808760-59002/matrix.json`
  - worker auth was copied and provider data/cache/home were isolated
  - local provider run ended before remote launch
  - provider thread id changed from
    `ses_13d2196a3ffeMIydmbf94IRqos` to
    `ses_13d217a20ffemYpoISSUcNi07g`
  - implication: OpenCode cannot be treated as credentials-only for thread
    transfer; the worker or slice must receive the provider-local OpenCode
    session state, or the kernel must reject the transfer instead of creating a
    fresh session silently
- Codex passed in
  `.artifacts/provider-thread-transfer/1781385834411-61445/matrix.json`
  - worker auth was copied and provider data/cache/home were isolated
  - local provider run ended before remote launch
  - provider thread id stayed
    `019ec2de-c98f-7440-b3e4-3edbdb3aa1ab`
  - worker recall marker was observed
  - implication: in this environment, Codex thread resume appears to be backed
    by provider-side state plus auth, not by local Codex data/cache state

Real local Docker slice save/restart evidence on June 13, 2026:

- OpenCode passed in
  `.artifacts/provider-thread-transfer/1781388631534-82761/matrix.json`
  - same Chariox agent record: `agent-3`
  - provider thread id stayed
    `ses_13cf63dabffexRi9Tby15Yk1bs`
  - `SaveSliceState(restart_agents)` stopped and restarted the slice-backed
    provider execution
  - post-restart recall marker was observed through the client kernel and
    terminal projection
- Codex passed in
  `.artifacts/provider-thread-transfer/1781388307332-60948/matrix.json`
  - same Chariox agent record: `agent-3`
  - provider thread id stayed
    `019ec307-7174-76d0-8a40-dba0ed637d88`
  - `SaveSliceState(restart_agents)` stopped and restarted the slice-backed
    provider execution
  - post-restart recall marker was observed through the client kernel path

Two implementation findings were necessary to make the real slice drill
meaningful:

- Slice relaunch must not reuse the previous `structured_endpoint`, because
  that endpoint points at the stopped worker-local provider server. Relaunch
  must clear it so the restarted worker spawns the provider again.
- Remote leased runtime projection needs to include the worker's
  `RuntimeProviderRun` snapshot. Codex only exposes the durable thread id after
  the first provider turn, so the home kernel must receive and project the
  worker run snapshot after launch. This required relay peer protocol version
  `3`.

Real live local-to-slice migration evidence on June 14, 2026:

- OpenCode passed in
  `.artifacts/provider-thread-transfer/1781420264462-60101/matrix.json`
  - same Chariox agent record: `agent-3`
  - local provider run `provider-run-1` was ended by `move_agent_to_remote`
  - provider thread id stayed
    `ses_13b13df17ffe46ychcDg5t4l0F`
  - the same agent moved from local execution to `slice:slice-1`
  - provider-local OpenCode state was copied into the slice before launch
  - post-migration recall marker was observed through the client kernel path
- Codex passed in
  `.artifacts/provider-thread-transfer/1781420327276-68599/matrix.json`
  - same Chariox agent record: `agent-3`
  - local provider run `provider-run-1` was ended by `move_agent_to_remote`
  - provider thread id stayed
    `019ec4ed-11e4-7383-98be-9b120ab8137a`
  - the same agent moved from local execution to `slice:slice-1`
  - provider-local Codex state was copied into the slice before launch
  - post-migration recall marker was observed through the client kernel path
- Claude Code passed in
  `.artifacts/provider-thread-transfer/1781419887303-27671/matrix.json`
  - same Chariox agent record: `agent-3`
  - local provider run `provider-run-1` was ended by `move_agent_to_remote`
  - provider thread id stayed
    `fc801906-14dd-4eaf-8869-af11fce0476b`
  - the same agent moved from local execution to `slice:slice-1`
  - Claude Code home state and `.claude.json` were copied into the slice before
    launch
  - post-migration recall marker was observed through the client kernel path

Earlier Claude Code runs exposed a separate local executable-resolution issue:
the daemon accepted `/opt/homebrew/bin/claude` even when it resolved to a
non-executable npm stub. The resolver now requires executable candidates and
can follow that stub layout to the platform-native Claude Code binary when it
is installed.

Additional implementation findings from the live migration drill:

- Moving an agent to a local Docker slice must resolve `slice:<id>` worker
  machine refs to the slice private relay config, not the public/default relay
  config.
- Starting a slice worker can take longer than the previous two-second private
  relay registration wait. The home connector now waits long enough for the
  worker and home peer registrations to converge.
- The moved agent must be attached to the slice store so slice save, cleanup,
  and delete operations understand that the agent now belongs to the slice.
- OpenCode must copy provider-local session state from the isolated drill
  provider data root. Copying the user's global OpenCode data by accident can
  both leak unrelated state into the slice and still fail same-session resume.
- Home-projected leased provider run ids must be namespaced by leased agent id.
  Worker kernels often reuse `provider-run-1`; projecting that id unchanged can
  collide with a previous local run on the home kernel.
- Live migration should not rely on provider-process teardown. Claude Code
  structured runs do not appear in the provider-process table, so
  `move_agent_to_remote` now terminates idle local provider runs through the
  kernel-owned provider-run lifecycle before binding the same agent to the
  worker.

Current executable conclusion:

```text
same provider thread, different local provider process: yes for OpenCode and Codex
same provider thread after same-host worker resume with shared state: yes for OpenCode and Codex
same provider thread after same-host worker resume with isolated worker state: yes for Codex, no for OpenCode
same provider thread after real slice save/restart with same Chariox agent record: yes for OpenCode and Codex
same provider thread after live migration from local unsliced execution into a slice: yes for OpenCode, Codex, and Claude Code
```

Harness findings that matter for later drills:

- Codex can report a running provider process before a provider thread id is
  available. The drill must require the thread id after the first provider
  turn, not before the first prompt.
- Local ChatGPT-backed Codex rejected `gpt-5.2` and `gpt-5.2-codex`; the drill
  uses `gpt-5.5` as the default Codex model in this environment unless
  `--provider-model codex=...` overrides it.
- The history outline API can omit a final display fragment while the raw
  JSONL history still contains the exact provider output. Marker observation
  should use unique prefixes for live waits and preserve raw JSONL artifacts
  for exact transcript evidence.
- Terminal-output pumping is not required for this local provider reload drill;
  kernel history and provider-run state are sufficient evidence. Later terminal
  fanout drills still need explicit terminal-event validation.
- The remote worker path that can accept a provider session id is currently the
  remote-native provider launch path (`native_tui: true`). A plain
  `LaunchProviderRun` for a remote-backed agent still falls through to local
  launch and fails with "agent is remote-backed and must launch its provider on
  the worker kernel."
- `move_agent_to_remote` still refuses any agent with a provider run. The
  same-host worker result therefore proves provider-thread portability, not the
  final autonomous "same agent becomes compliant" operation.
- OpenCode resumes by session id only when the session exists in the worker's
  provider-local state. If it is missing, the existing binding creates a fresh
  session. The final transfer implementation must make that fallback
  non-silent for server-compliance moves.
- Codex passed a credentials-only isolated worker resume in this environment
  and then passed the real slice save/restart drill. The real slice pass
  required the home kernel to project the worker provider run snapshot from
  relay peer protocol version `3`.
- OpenCode passed the real slice save/restart drill when provider-local slice
  state was preserved across `SaveSliceState(restart_agents)`.
- The real slice save/restart drill proves continuity inside a slice lifecycle.
  The live migration drill now separately proves the stronger local-unsliced to
  slice transfer path for Codex, OpenCode, and Claude Code.
- Claude Code validation depends on a working platform-native Claude Code
  binary. A stale npm stub can exist at `/opt/homebrew/bin/claude`; the kernel
  now rejects non-executable stubs during resolution.

## Drill 0: Provider State Inventory

Purpose: identify the provider-local state required to resume a thread inside a
fresh slice.

For each provider:

1. Start a local Chariox agent.
2. Send a prompt that creates a unique marker in the provider thread.
3. Capture the provider thread id from kernel/provider state.
4. Stop the provider process.
5. Attempt to resume the same provider thread after moving only credentials
   into an isolated environment.
6. If resume fails, incrementally add provider-local state until resume works.

Pass criteria:

- the required provider-local state paths are identified
- credentials-only success or failure is recorded
- the provider's failure mode for missing state is recorded
- the required state can be copied without exposing secrets in logs

Provider-specific questions:

- Codex: does `thread/resume` work in a slice with only the thread id, or does
  it require local rollout/thread files?
- OpenCode: what local session data is required beyond the session id?
- Claude Code: what local project/session files are required beyond
  credentials?

## Drill 1: Baseline Local Reload Preserves Thread

Purpose: verify the existing provider reload path preserves provider thread id
when only launch-time config changes locally.

For each provider:

1. Start a local agent.
2. Send a prompt with a unique marker.
3. Capture provider thread id.
4. Trigger an MCP grant or revoke that requires provider reload.
5. Wait for reload and automatic continuation behavior where applicable.
6. Send a follow-up prompt asking the provider to recall the marker.
7. Capture provider thread id after reload.

Pass criteria:

- pre- and post-reload provider thread ids match
- the provider recalls the marker
- no blank replacement thread is started
- terminals remain attached and show coherent prompt/output lifecycle

This drill establishes the baseline for "same provider thread, different
provider process."

## Drill 2: Duplicate-Run Prevention

Purpose: prove the transfer machinery never leaves two live provider runs for
the same Chariox agent and provider thread.

For each provider:

1. Start a local agent and capture provider thread id.
2. Begin a transfer operation.
3. Instrument provider store and process tracker state during the transition.
4. Attempt to send input to the old run after the new run is ready.
5. Attempt to send input to the new run.

Pass criteria:

- old run is stopped, parked, or rejected before new run accepts prompts
- only one active provider run is associated with the Chariox agent
- input to the old run fails loudly or is routed to the new active run by
  explicit kernel state, not by accident

## Drill 3: Same-Host Worker Resume

Purpose: validate same-thread resume across a worker boundary before adding
slice-specific isolation.

For each provider:

1. Start a home kernel and worker kernel on the same host.
2. Start a local home-backed agent and create a unique marker.
3. Capture provider thread id and required provider-local state.
4. Transfer execution to the worker using a leased-agent path that accepts
   provider resume state.
5. Launch provider execution on the worker with the same provider thread id.
6. Send a prompt through the home/client kernel and attached terminal.

Pass criteria:

- worker provider run reports the same provider thread id
- provider recalls the marker
- home/client kernel terminal fanout remains coherent
- no duplicate local provider run remains active

This drill separates "remote execution resume" from Docker slice filesystem and
isolation effects.

## Drill 4: Standard Slice Resume

Purpose: validate the actual Chariox Server compliance scenario.

For each provider:

1. Start a local autonomous agent outside the slice.
2. Create a unique marker in the provider thread.
3. Resolve a test Chariox Server policy requiring the standard slice.
4. Ask the client kernel to make the agent compliant.
5. Prepare the standard slice with credentials and required provider-local
   state identified in Drill 0.
6. Relaunch provider execution inside the slice with the same resume state.
7. Verify same provider thread id.
8. Invoke a test workflow endpoint from a server-kernel simulation.
9. Confirm the client kernel fans out to the slice provider run and attached
   Chariox terminals.

Pass criteria:

- same provider thread id before and after slice transfer
- provider recalls pre-transfer marker
- server-style workflow invocation reaches the client kernel, then the slice
  provider thread
- terminal fanout remains coherent
- server admission is only completed after transfer verification

## Drill 5: Missing-State Negative Test

Purpose: prove failed transfer does not silently create a new provider thread.

For each provider:

1. Start a local agent and capture provider thread id.
2. Intentionally omit required provider-local state in the slice.
3. Attempt transfer.
4. Observe provider launch or resume failure.
5. Verify kernel rollback behavior.

Pass criteria:

- transfer fails with an explicit same-thread verification error
- no blank provider thread is accepted as success
- old local configuration is restored exactly
- user/agent-visible error explains that same-thread slice transfer failed

## Drill 6: Compliance Downgrade And Exact Restore

Purpose: validate the autonomous-agent downgrade/restore contract.

For each provider:

1. Start an autonomous agent with a richer local configuration.
2. Capture full config snapshot: MCPs, skills, scripts, connectors,
   permission level, execution mode, write access mode, workspace, slice state,
   model, variant, and provider thread id.
3. Apply a server compliance requirement such as "slice required, skills only,
   no MCP/script/connector tools."
4. Verify the effective downgraded configuration.
5. Disconnect from the server.
6. Restore automatically.

Pass criteria:

- downgraded config satisfies server requirement
- provider thread id is preserved where the scenario requires preservation
- automatic restore returns exactly to the previous saved config
- no additional upgrade or new grant occurs
- manual user grants after restore still work through normal paths

## Drill 7: Workflow Endpoint Trigger After Transfer

Purpose: verify the final Chariox Server trigger path, not just provider resume.

Topology:

```text
test app service
  -> server kernel app bridge simulation
  -> server workflow endpoint
  -> client kernel
  -> transferred provider thread
  -> attached Chariox terminals
```

Steps:

1. Connect a compliant transferred agent to a test server session.
2. Create a workflow node for that client agent.
3. Create an endpoint for the node.
4. Invoke the endpoint with structured test input.
5. Validate the structured output.
6. Observe terminal fanout on the client side.

Pass criteria:

- structured input reaches the provider thread through the client kernel
- attached terminals show the turn lifecycle
- output validates against schema
- server kernel receives the result through workflow runtime
- app service receives a completion event

## Drill 8: Provider Matrix Report

Purpose: turn the drill results into a product decision.

For each provider and scenario, record:

```text
provider
provider version
local reload preserves thread: yes/no
same-host worker transfer preserves thread: yes/no
slice transfer preserves thread: yes/no
required state paths
known failure modes
rollback reliable: yes/no
recommended V1 policy
```

Recommended V1 policy values:

- `supports_live_slice_transfer`
- `must_start_in_slice_for_strict_servers`
- `unsupported_for_strict_autonomous_servers`

## Final Decision Rule

If all three providers pass Drill 4 and Drill 5 with a local-unsliced starting
point, Chariox Server V1 can support live autonomous compliance transfer into a
standard slice.

If one or more providers fail, Chariox Server V1 should still support strict
server policies by requiring autonomous agents to start inside the standard
slice before connecting to servers that require slice-backed execution.

The current Codex, OpenCode, and Claude Code evidence supports live
local-to-slice migration for those providers, with explicit provider-state
transfer and same-thread verification.

The remaining gating evidence is the negative/rollback side of the matrix:
missing required provider-local state must fail explicitly and must not be
accepted as a fresh blank provider thread.
