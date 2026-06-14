# Arroba Server Provider Thread Round-Trip Plan

## Goal

Promote the validated local-to-slice provider-thread transfer path into a
kernel/runtime capability and add the reverse path back to local execution.

This is not a server-policy feature and does not require UX exposure. The
capability is only about moving the same Arroba agent record between local and
slice execution while preserving the same provider-native conversation thread.

The required round trip is:

```text
local provider thread
  -> transfer same Arroba agent to slice
  -> continue the same provider thread in the slice
  -> exchange prompts in the slice
  -> transfer same Arroba agent back to local execution
  -> continue the same provider thread locally
  -> restore the exact original local agent configuration
```

## Definitions

Provider thread means the provider-native conversation identity:

- Codex: Codex thread id
- OpenCode: OpenCode session id
- Claude Code: Claude session id

Same agent means the same Arroba agent record. The Arroba provider run id and
provider process may change. The provider thread id must not change.

Original configuration means the local agent state before the forward transfer:

- provider
- model
- variant
- effort
- execution mode
- permission level
- write access mode
- workspace and worktree
- extension grants
- MCP, skill, script, and connector visibility
- provider resume state
- provider-local state roots needed to resume the provider thread
- local versus remote execution binding

## Product Capability

Implement a runtime capability with two directions:

```text
transfer_agent_execution_preserving_provider_thread(local -> slice)
transfer_agent_execution_preserving_provider_thread(slice -> local)
```

The first implementation can be an internal kernel/service operation. It does
not need CLI, web, server-policy, or UX exposure.

The operation must be kernel-owned because the kernel owns sessions, agents,
provider runs, prompt lifecycle, terminal projection, remote execution leases,
and slice attachment state.

## Forward Path: Local To Slice

The forward operation must:

1. Resolve the Arroba agent and verify the caller owns it.
2. Require the agent to be idle. If there is an active provider prompt, reject
   the transfer.
3. Snapshot the original local configuration.
4. Capture the active provider resume state and provider thread id.
5. Prepare and start the target slice.
6. Copy provider-local state into the slice.
7. Import provider auth into the slice.
8. Move the same Arroba agent record to the slice worker.
9. Terminate the idle local provider run through the kernel-owned provider-run
   lifecycle.
10. Launch provider execution in the slice with the captured resume state.
11. Verify the slice provider run reports the same provider thread id.
12. Persist a transfer record so the reverse path can restore exactly.

The operation must not rely on provider-process teardown. Claude Code
structured runs do not appear in the provider-process table, so provider-run
lifecycle termination is the correct authority.

## Reverse Path: Slice To Local

The reverse operation must:

1. Resolve the same Arroba agent record.
2. Require the agent to be idle. If there is an active provider prompt, reject
   the reverse transfer.
3. Load the transfer record created by the forward path.
4. Capture the current slice provider resume state and provider thread id.
5. Terminate the slice provider run through the kernel-owned provider-run
   lifecycle.
6. Copy provider-local state back from the slice to the correct local provider
   state location.
7. Clear the remote execution binding from the same Arroba agent record.
8. Restore the original local agent configuration exactly.
9. Launch the local provider with the current resume state.
10. Verify the local provider run reports the same provider thread id as both:
    the original pre-transfer local thread id and the pre-reverse slice thread
    id.
11. Confirm no duplicate live provider run remains for the same Arroba agent and
    provider thread.

Reverse is not just clearing `remote_execution`. Provider-local state produced
inside the slice must be copied back when the provider requires it.

## Provider State Transfer

Extract provider-state transfer out of the drill into a provider-specific
runtime/service layer.

Each provider needs a state transfer manifest:

- local source paths
- slice destination paths
- slice source paths for reverse transfer
- local destination paths for reverse transfer
- auth import requirements
- secret-safe logging rules
- missing-state behavior
- whether credentials-only resume is valid

The first providers are:

- Codex
- OpenCode
- Claude Code

OpenCode is known to require provider-local session state. Codex may resume
with less in the current environment, but the implementation should still use
explicit provider-state transfer for consistency and determinism. Claude Code
requires Claude home state and `.claude.json` in the validated drill.

## Transfer Record

The forward operation must persist a transfer record that is sufficient for an
exact reverse restore.

The record should include:

- transfer id
- session id
- agent id
- original local configuration snapshot
- original provider thread id
- original provider resume state
- target slice id and worker kernel id
- leased agent id
- active worker provider run id after slice launch
- provider state transfer manifest version
- provider-local state paths used for forward transfer
- timestamp and status

The reverse operation should mark the record restored only after the local
provider thread has been relaunched and verified.

## Verification Contract

Every transfer must verify the provider thread id.

If the launched provider reports a different thread id:

1. terminate the bad new provider run
2. do not accept the transfer as successful
3. preserve or restore the last known good execution side when possible
4. return a hard error containing the expected and actual provider thread ids

The kernel must never silently accept a fresh blank provider thread as success.

## Failure Handling

Forward failure:

- if slice preparation fails, keep the local run/config unchanged
- if slice launch fails after the local run was terminated, relaunch locally
  from the captured resume state when possible
- if thread verification fails, terminate the slice run and restore local
  execution from the original snapshot

Reverse failure:

- if local relaunch fails, keep the transfer record open
- if local relaunch reports a different thread id, terminate the bad local run
  and relaunch the slice run from the captured slice resume state when possible
- if exact local config restore fails, keep the agent marked as not fully
  restored and expose the mismatch in the operation result

## Round-Trip Drill

Add a new executable drill mode:

```bash
node apps/cli/scripts/live-provider-thread-transfer-drill.mjs \
  --drill live-migrate-roundtrip-slice \
  --provider <provider> \
  --timeout-ms 900000 \
  --slice-build-image auto \
  --keep-slice-on-failure
```

The drill should:

1. Start a local kernel and session.
2. Spawn a local agent.
3. Launch the local provider run.
4. Send marker A locally.
5. Capture the local provider thread id and original config snapshot.
6. Transfer local to slice using the product operation.
7. Verify the slice provider thread id matches.
8. Ask the slice provider to recall marker A.
9. Send marker B in the slice.
10. Reverse transfer slice to local using the product operation.
11. Verify the local provider thread id still matches.
12. Ask the local provider to recall marker A and marker B.
13. Verify the final local config exactly matches the original snapshot.
14. Verify no duplicate local or slice provider run remains active.

## Provider Matrix

Run the round-trip drill for:

- OpenCode
- Codex
- Claude Code

Pass criteria for every provider:

```text
same Arroba agent id before, during, and after transfer
same provider thread id local -> slice
same provider thread id slice -> local
local provider recalls marker A before transfer
slice provider recalls marker A after forward transfer
slice provider creates marker B
local provider recalls marker A and marker B after reverse transfer
final local config equals original config snapshot
no duplicate live provider runs remain
```

## Documentation Updates

After implementation and validation:

1. Update `docs/ARROBA_SERVER_PROVIDER_THREAD_TRANSFER_DRILLS_PLAN.md` with the
   round-trip evidence and artifact paths.
2. Update `docs/ARROBA_SERVERS_AND_ARROBANET_CONCEPT.md` to say the provider
   thread transfer path supports round-trip restore for the providers that pass
   the matrix.
3. Record provider-specific state transfer requirements in the drill plan.

## Current Status

As of June 14, 2026, the positive one-way local-to-slice path is validated for
OpenCode, Codex, and Claude Code. The reverse slice-to-local path is not yet
implemented. The next work is to promote the drill orchestration into a
kernel/service capability and add the round-trip drill.
