# Implementation Invariants

This checklist is the merge gate for M4.5 runtime slices before the final I/O-coordination design. It is intentionally operational: every item must be answerable from code, tests, health output, or explicit documentation.

## Ownership

- Mutable prompt lifecycle changes enter through `KernelAgentService` or the `AgentRuntime` mailbox path and mutate the shared `PromptStateOwner` before any compatibility session mirror is refreshed. `RuntimeSession`/`PromptRuntimeState` prompt fields are compatibility mirrors, not admission, routing, or lifecycle authorities.
- Mutable session lifecycle changes enter through `KernelSessionService` or the `SessionRuntime` mailbox path. Public session creation, attach, detach, focus, cycle, resize, config, alias, end, and delete must not add new direct request-handler mutation paths.
- Workflow progression enters through workflow runtime methods or scheduler-owned dispatch paths. New workflow state transitions must not bypass workflow lane admission.
- Provider-run structured submit, abort, output polling, selection sync, and teardown must remain behind provider runtime lanes or provider-run actor methods.
- `DaemonApp` may remain bootstrap/shutdown/test composition during M4.5, but it must not gain new command-state ownership. New hot-path behavior must declare whether it is actor-owned or projection-owned; "compatibility-only" is allowed only for an explicitly named deletion step in the current cutover slice.
- A cutover slice that adds an owned runtime path must delete the matching app-backed helper before merging. Do not preserve a second compatibility path after the owner path is covered by tests.

## Projection Refresh

- Any response that returns an authoritative `RuntimeSession` must refresh `SessionStateProjectionStore`; if prompt state can change, it must also refresh `AgentRuntimeProjectionStore`.
- Prompt submit, completion, cancellation, dispatch failure, queue advancement, provider settlement, and detach cleanup must publish prompt projections from the authoritative session returned by the mutation boundary.
- Session delete/end cleanup must remove session projections, agent-runtime projections, agent lanes, workflow lanes, focused-agent projection entries, and history projection entries through one ordered path.
- Provider-run lifecycle changes must refresh or invalidate provider-run and provider-process projections before warmed reads can reuse them.
- `GetDaemonHealth.projection_invariants.mismatches` must stay empty in normal operation. A non-empty mismatch is a correctness bug unless the command is explicitly testing stale-state detection.

## Cleanup

- Any session close/delete path must clean up provider runs, terminal buffers, prompt workspace claims, workflow lanes, agent lanes, and warmed projections.
- Any prompt terminal path, including completion, cancellation, provider dispatch failure, idle settlement, and session cleanup, must release provider prompt workspace claims.
- Provider runtime cleanup must preserve tombstone/generation checks so slow provider I/O cannot restore stale runtime state.
- Workflow blocked-on-claim retries must be triggered only after the relevant claim releases, and terminal workflow states must not keep retry registrations alive.

## Overload

- Actor and runtime queues must be bounded. New hot-path mailboxes must expose queue snapshots in daemon health or explain why an existing health snapshot covers them.
- Enqueue failure must return a daemon error on the command path that owns cleanup. It must not be logged and swallowed when the caller needs to release claims, publish notices, or retry.
- Slow reads and background I/O must not block prompt submit/cancel/complete, session attach/focus/resize, or daemon health in warmed cases.
- Terminal output, notices, and completion buffers must remain bounded per recipient, with health counters for backlog and trimming.

## Health

- `GetDaemonHealth` must expose enough signal to debug actor queues, provider-run actor enqueue pressure, capability executor pressure, terminal backlog, transport pressure, provider catalog cache state, workspace claims, and projection drift.
- Legacy prompt-count fields may mirror canonical agent-runtime counts for wire compatibility, but the canonical source must be documented.
- Health reads must avoid the compatibility app lock whenever the relevant stores are warm or cloned.

## Tests

- Every migration slice must include a focused regression test for the ownership, projection, cleanup, or overload behavior it changes.
- Projection-first reads need tests that hold the compatibility app lock and assert the warmed path still completes.
- Cleanup paths need tests for both normal success and failure/cancellation paths when claims, lanes, projections, or provider state are involved.
- Queue/overload behavior needs tests that fill the relevant lane and assert the command is either admitted through the correct mailbox or rejected with an explicit overload error.
- Documentation updates must land in the same slice as behavior changes, including `TASKS.md` and `PROGRESS_LOG.md`, and must state whether the slice deletes an app-backed compatibility path or leaves one named blocker behind.
