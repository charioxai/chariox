# Arroba Server Provider Thread Transfer Drills Plan

## Goal

Validate whether an autonomous Arroba agent can be made compliant with an
Arroba Server slice requirement while preserving the same provider-native
thread.

This plan answers one open design question:

```text
Can a running agent move from local execution into a standard Arroba slice and
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

Same agent means the same Arroba agent record continuing through the same
provider-native thread. The provider process and Arroba provider run id may
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
- Arroba terminals attached to the session continue to receive coherent output
- server-directed workflow endpoint invocation still reaches the client kernel
- no duplicate live provider run exists for the same Arroba agent and provider
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
slice-backed server admission to start inside the standard Arroba slice instead
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

## Drill 0: Provider State Inventory

Purpose: identify the provider-local state required to resume a thread inside a
fresh slice.

For each provider:

1. Start a local Arroba agent.
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
the same Arroba agent and provider thread.

For each provider:

1. Start a local agent and capture provider thread id.
2. Begin a transfer operation.
3. Instrument provider store and process tracker state during the transition.
4. Attempt to send input to the old run after the new run is ready.
5. Attempt to send input to the new run.

Pass criteria:

- old run is stopped, parked, or rejected before new run accepts prompts
- only one active provider run is associated with the Arroba agent
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

Purpose: validate the actual Arroba Server compliance scenario.

For each provider:

1. Start a local autonomous agent outside the slice.
2. Create a unique marker in the provider thread.
3. Resolve a test Arroba Server policy requiring the standard slice.
4. Ask the client kernel to make the agent compliant.
5. Prepare the standard slice with credentials and required provider-local
   state identified in Drill 0.
6. Relaunch provider execution inside the slice with the same resume state.
7. Verify same provider thread id.
8. Invoke a test workflow endpoint from a server-kernel simulation.
9. Confirm the client kernel fans out to the slice provider run and attached
   Arroba terminals.

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

Purpose: verify the final Arroba Server trigger path, not just provider resume.

Topology:

```text
test app service
  -> server kernel app bridge simulation
  -> server workflow endpoint
  -> client kernel
  -> transferred provider thread
  -> attached Arroba terminals
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

If all three providers pass Drill 4 and Drill 5, Arroba Server V1 can support
live autonomous compliance transfer into a standard slice.

If one or more providers fail, Arroba Server V1 should still support strict
server policies by requiring autonomous agents to start inside the standard
slice before connecting to servers that require slice-backed execution.

The implementation plan should be written only after this matrix is known.

