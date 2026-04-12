# M4.5 Kernel Runtime Refactor Plan

## Goal

Move the daemon from a request-handler-owned `DaemonApp` mutex toward an actor/event/projection kernel that can stay responsive while prompts, provider output, workflow scheduling, relay traffic, history reads, capability jobs, and health inspection all run concurrently.

The target is not "more locks." The target is explicit ownership:

- commands enter through a router
- actors or single-owner services mutate state
- kernel events record ordered runtime facts
- projections serve reads
- background work cannot block interactive commands

## Current Baseline

The current primary CLI path uses the kernel WebSocket transport. The first M4.5 slices now normalize requests through `KernelCommand`, route them through `CommandRouter`, publish bounded replay events through `EventLog`, and keep the responsiveness-critical operations on a bounded `InteractiveCommandLane`.

The daemon is no longer just the old request-handler surface with new names. Provider-run structured submit/cancel/poll now runs through per-run actors, command-id retries fan out instead of double-dispatching, slow consumers and inbound request bursts have bounded handling, session lifecycle/focus/resize behavior is collected under `KernelSessionService`, and prompt submit/cancel/complete/queue-advance lifecycle behavior is collected under `KernelAgentService`. Prompt submit/cancel commands now enter per-agent mailboxes, session attach/detach/focus/cycle/resize/end/delete commands enter per-session mailboxes instead of the generic interactive queue, and the session runtime publishes a focused-agent projection used by agent routing once focus is warm. Agent lifecycle responses that change focus refresh the same projection. The router also has the first projection stores for warmed `GetSessionState`, `ListSessions`, `GetSessionHistory`, `GetProviderRun`, `ListProviderProcesses`, and `GetProviderCatalog` reads, and prompt lifecycle mutations now publish session snapshots into the shared session projection.

Important caveat: the implementation still has a compatibility `DaemonApp`, and several hot paths still pass through shared app state while they migrate. M4.5 is in progress, not complete.

Current event replay is a bounded in-memory recent-event buffer. It supports reconnect-friendly local behavior, but it is not yet a durable event log.

Current multi-agent reliability work has already established one important invariant: focus is UI state, while prompt execution and provider-run liveness are per-agent. M4.5 must preserve that invariant by treating session-global active prompt/run fields as compatibility projections, not as ownership.

## Implementation Status

Status as of 2026-04-12:

- Landed: `KernelCommand` and `KernelEvent` envelopes with command ids, causation/correlation metadata, and priority classification.
- Landed: `EventLog` service with daemon-local event ids, stream sequence ids, bounded replay windows, and explicit replay-gap reporting.
- Landed: `SessionSnapshotProjection` skeleton with projection metadata for client boot/reconciliation.
- Landed: `CommandRouter` facade and bounded `InteractiveCommandLane` for prompt submit/cancel, focus/cycle, attach/detach, resize, and related session commands.
- Landed: safe command-id retry handling, including in-flight duplicate fanout and duplicate-conflict rejection when a reused command id carries a different request fingerprint.
- Landed: CLI typed `replay_gap` handling with a concise user-visible refresh notice.
- Landed: inbound WebSocket request admission bounds before task spawn.
- Landed: local IPC compatibility routing through the kernel command normalization path.
- Landed: provider-run actors for structured provider submit, abort, and output polling, including runtime-slot tombstones/generations that prevent cleanup races from restoring stale provider runtime state.
- Landed: provider output polling no longer holds provider-family global locks while performing provider I/O.
- Landed: reserved-listener WebSocket integration harness startup to remove the observed free-port race.
- Landed: `KernelSessionService` now owns attach, detach, end, delete-by-ref, focus/cycle, and terminal resize behavior behind the public `DaemonApp` compatibility methods.
- Landed: `KernelAgentService` now owns prompt submit, kernel submit acknowledgement/dispatch preparation, cancel, runtime cancel, completion, queue advancement, and cancellation finalization behind the public `DaemonApp` compatibility methods.
- Landed: `AgentRuntime` per-agent command mailboxes for prompt submit/cancel admission, so agent-scoped prompt commands are not rejected behind unrelated generic interactive work.
- Landed: `SessionRuntime` per-session command mailboxes for attach/detach/focus/cycle/resize/end/delete admission, so session-scoped UI and lifecycle commands are isolated from the generic interactive queue and from other sessions.
- Landed: session mailbox cleanup on successful end/delete, so closed session lanes do not stay registered indefinitely.
- Landed: `DaemonHealthProjection` with session/agent mailbox queue snapshots, provider runtime operation-lane pressure, session projection counts, active/queued prompt counts, and provider-catalog cache status exposed through `GetDaemonHealth`.
- Landed: `FocusedAgentProjection` shared by `SessionRuntime`, `AgentRuntime`, and router-side agent lifecycle refreshes, so focus changes captured by the session mailbox or local agent spawn/destroy responses let untargeted prompt submit/cancel route to the focused agent without synchronously taking the compatibility app lock for focus lookup.
- Landed: `SessionStateProjectionStore` for warmed session/list/prompt-state snapshots. `GetSessionState` and warmed `ListSessions` can return projected session data without taking the compatibility app lock after the projection has been warmed by list responses, prompt/session/agent/workflow responses, prompt lifecycle publications, or transitional post-mutation snapshots.
- Landed: `SessionHistoryProjectionStore` for warmed transcript history. `GetSessionHistory` can use a warmed session snapshot to load disk history without taking the compatibility app lock, then serve repeated warmed transcript reads from memory while successful appends keep the warmed projection current.
- Landed: `ProviderRunProjectionStore` for warmed provider-run snapshots. `GetProviderRun` can return without taking the compatibility app lock after launch/read warm-up, and start/finish/fail/park/resume/end lifecycle updates refresh the shared projection.
- Landed: `ProviderProcessProjectionStore` for warmed process-list snapshots. `ListProviderProcesses` can return without taking the compatibility app lock after warm-up, and provider-run/session lifecycle changes invalidate the snapshot so teardown-safety metadata is recomputed before reuse.
- Landed: prompt-state projection publication. Prompt submit, completion, cancellation, dispatch failure, and queue advancement now publish updated session snapshots into the shared projection, so warmed `GetSessionState` reflects prompt lifecycle transitions without a compatibility-store read.
- Landed: `ProviderCatalogProjectionStore` for TTL-bound warmed provider catalog snapshots. `GetProviderCatalog` can return without taking the compatibility app lock after warm-up, and provider logout/configuration changes invalidate the projection.
- Landed: projection correctness hardening. Provider-process snapshots are canonical rather than request-scoped, teardown refreshes affected process/run projections, warmed OpenCode provider-run reads preserve selection-sync side effects, relay reconfiguration invalidates provider-catalog projection state, and agent command lanes are cleaned up on agent/session removal.
- Landed: transport health projection. Kernel websocket slow-consumer closes, outgoing queue overflows, inbound overload rejections, replay gaps, request/event counts, active connections, and active subscriptions are now surfaced through `GetDaemonHealth`.

Still open:

- move `KernelSessionService` and `KernelAgentService` state out of the compatibility facade and into the new `SessionRuntime`/`AgentRuntime` mailbox owners
- move prompt queues and per-agent prompt state out of the shared session store into actor-owned state/projections
- broaden actor-owned projections beyond focused-agent routing and warmed session/list/history/provider-run/process/prompt-state/provider-catalog snapshots so remaining provider/read models can be served without compatibility-store reads on the hot path
- remove remaining hot request paths that require `Arc<Mutex<DaemonApp>>`
- expand `DaemonHealthProjection` beyond actor/projection/cache/transport state to include workspace coordination
- introduce `WorkspaceCoordinator` claim enforcement for worktree/file/port collisions

## Non-Goals

- Do not rewrite provider adapters for every provider family.
- Do not make relay a workspace authority.
- Do not introduce a generic agent endpoint transport before the OpenCode-first path is stable.
- Do not move workflow mailboxing into the interactive control subsystem.
- Do not require daemon restart recovery beyond the persistence guarantees that already exist, unless this plan explicitly adds a persisted store in a later slice.

## Terminology

Use these terms consistently:

- `InteractiveCommandLane`: the priority kernel lane for user-visible operations that must remain responsive, including prompt submit, cancel, focus, attach, detach, resize, and subscription resume.
- `ControlService`: the provider/runtime control subsystem described in `docs/CONTROL_CHANNEL_PLAN.md`, including provider-run cancellation/interrupt, workflow runtime tools, attachment, memory update, and compaction operations.
- `EventLog`: the ordered stream of kernel facts used for live replay, projection updates, client reconciliation, and later relay resume.
- `ProjectionStore`: materialized read models derived from actor state and kernel events.

Do not call the `InteractiveCommandLane` the "control lane." That name is reserved for provider/runtime control operations.

## Kernel Command Contract

Every mutating request that enters the kernel should be normalized to a `KernelCommand`.

Minimum envelope:

```text
KernelCommand
  command_id: string
  command_type: string
  submitted_at_ms: u64
  source: local_cli | local_ipc | relay_client | relay_peer | daemon_background
  session_id?: string
  attachment_id?: string
  agent_id?: string
  provider_run_id?: string
  workflow_run_id?: string
  node_run_id?: string
  idempotency_key?: string
  causation_id?: string
  correlation_id: string
  priority: interactive | normal | background
  payload: object
```

Rules:

- `command_id` is stable for one submitted command and is echoed in accepted/rejected/completed events.
- `correlation_id` groups all work caused by one user action, workflow trigger, or relay request.
- `causation_id` points to the command or event that directly caused this command.
- Duplicate `command_id` handling must be idempotent for commands that may be retried by reconnecting clients.
- Commands validate before actor dispatch. Invalid commands emit or return a structured rejection and must not partially mutate state.

First commands to migrate:

1. `prompt.submit`
2. `prompt.cancel`
3. `agent.focus`
4. `terminal.resize`
5. `session.attach`
6. `session.detach`
7. `event.subscribe`
8. `event.resume`

These are the responsiveness-critical path.

## Kernel Event Contract

Every runtime fact that clients, projections, replay, or relay resume need should be represented as a `KernelEvent`.

Minimum envelope:

```text
KernelEvent
  event_id: u64
  stream_id: string
  stream_seq: u64
  event_type: string
  recorded_at_ms: u64
  command_id?: string
  causation_id?: string
  correlation_id: string
  session_id?: string
  attachment_id?: string
  agent_id?: string
  provider_run_id?: string
  workflow_run_id?: string
  node_run_id?: string
  payload: object
```

Rules:

- `event_id` is daemon-local and monotonically increasing within one daemon process.
- `stream_id` scopes replay. Use `session:<session_id>` for session events and `daemon` for daemon-wide status events.
- `stream_seq` is monotonic within one `stream_id`.
- Events must be append-only. Corrections are new events, not in-place mutation.
- Projections record the last applied `event_id` and `stream_seq`.
- Client reconciliation keys are `command_id`, `event_id`, `agent_id`, and prompt text where prompt echo compatibility is still needed.

M4.5 replay policy:

- The first slice may keep the event log in memory, but the buffer must be modeled as an `EventLog` service rather than transport-local storage.
- Replay windows must be explicit per stream.
- If a client resumes from a cursor older than the retained replay window, the kernel must return a replay-gap response and send a fresh projection snapshot.
- The docs and wire types must not call events durable until they survive daemon restart.

Durable replay policy, later slice:

- Persist event streams or persist enough projection checkpoints plus tail events to reconstruct client state after daemon restart.
- The relay milestone may depend on live reconnect/replay, but must not assume daemon-restart durability unless that slice has landed.

## Projection Store

Clients should read projections for boot, refresh, inspection, and reconnect recovery. Events keep those projections fresh.

Minimum projections:

| Projection | Scope | Required Fields |
|------------|-------|-----------------|
| `SessionListProjection` | daemon | sessions, aliases, status, focused agent summary, last activity, last applied event id |
| `SessionSnapshotProjection` | session | session metadata, attachments, focused agent, prompt states, compatibility active prompt/run fields, config state |
| `AgentRuntimeProjection` | session + agent | agent metadata, provider run binding, prompt work state, liveness, activity label, queue depth |
| `TranscriptPageProjection` | session + agent | cursor, ordered transcript entries, merge keys, source provider run ids |
| `WorkflowInspectionProjection` | session + workflow/run | definitions, endpoints, run status, node states, mailbox/handoff summaries, failure/audit state |
| `ProviderLedgerProjection` | daemon/session | provider processes, provider runs, pid/endpoint mode, owner run ids, teardown state |
| `DaemonHealthProjection` | daemon | active requests, actor queue depths, background jobs, slow consumers, provider discovery state, relay state |

Projection rules:

- Every projection includes `projection_version`, `last_event_id`, and `generated_at_ms`.
- Projection reads must not require whole-session refreshes on the interactive path.
- A command response may include an updated projection when that avoids a round trip, but the event stream remains the reconciliation path.
- Optimistic CLI state is allowed only when it can be reconciled by `command_id` or by a matching event/projection version.

## Actor Ownership Matrix

| Operation or State | Owner | Notes |
|--------------------|-------|-------|
| session lifecycle, attachments, focused agent | `SessionActor` | Focus changes must not mutate provider-run liveness in multi-agent sessions. |
| prompt queues and prompt state | `AgentActor` for agent-scoped work, coordinated by `SessionActor` | Session-global active/queued fields are compatibility projections. |
| provider process/run integration | `ProviderRunActor` | Normalizes OpenCode/Codex/native provider events into kernel events. |
| terminal output fanout | `AgentActor` + transport gateway | Fanout must not block provider event ingestion. |
| workflow run progression | `WorkflowRunActor` | Owns mailbox delivery, barriers, node activation, failure state, watchdog interaction. |
| capability jobs | `CapabilityExecutor` | Bounded queues. Reports progress/results through events. |
| file/worktree claims | `WorkspaceCoordinator` | Enforces collision prevention before code-writing capability/provider work. |
| relay connection state | `RelayRuntime` | Owns registration, remote subscriptions, and relay I/O without becoming workspace authority. |
| provider catalogs/history scans/health reads | background executors + projections | Never run on the interactive command lane. |

## Interactive Command Lane

The `InteractiveCommandLane` handles operations where the user expects immediate feedback.

Interactive commands:

- prompt submit
- prompt cancel
- focus/cycle
- terminal resize
- session attach/detach
- event subscribe/resume
- session delete acknowledgement, before teardown continues in background

Policy:

- Interactive commands use bounded queues with a small, explicit limit.
- If the lane is full, reject new commands with a retryable overload error rather than waiting behind background work.
- Background jobs must not hold actor access needed by interactive commands while doing I/O.
- Cancellation should preempt normal prompt work for the same agent where provider semantics allow it.
- Resize and focus commands may coalesce; prompt submit and cancel must not.

## Backpressure and Slow Consumers

Required policies:

- Each subscription has an outbound queue limit.
- A slow subscriber is disconnected with a structured retryable reason.
- Disconnecting one subscriber must not block provider output ingestion or other subscribers.
- Recent-event retention is per stream and bounded.
- A replay gap triggers projection resync, not silent success.
- Background executors have separate queue limits and health projection counters.

## Worktree and Collision Coordination

M4.5 should introduce the runtime boundary for workspace coordination even if full merge automation lands later.

Minimum `WorkspaceCoordinator` responsibilities:

- allocate or validate `WorktreeAssignment`
- track active worktree/file/port claims
- reject concurrent code-writing work in the same active worktree when claims conflict
- expose claim state in `DaemonHealthProjection` or a dedicated coordination projection
- release stale claims on provider-run settlement, cancellation, session delete, or explicit recovery

Rules:

- Workflow node dispatch consults the coordinator before starting a code-writing node.
- Capability jobs that mutate files consult the coordinator.
- Provider-run placement records the assigned worktree in runtime state.
- Shared-session worktree mode remains allowed for single-agent or explicitly serialized work.

## Migration Plan

### Phase 0. Contract Freeze

- Add `KernelCommand` and `KernelEvent` Rust types behind conversion adapters.
- Add `command_id`, `correlation_id`, and optional `causation_id` to WebSocket request handling.
- Keep existing `LocalDaemonRequest` shapes externally while normalizing internally.

### Phase 1. EventLog Service

- Move recent-event storage out of `kernel_transport.rs` into an `EventLog` service.
- Add stream ids, per-stream sequence ids, replay-window metadata, and replay-gap responses.
- Keep current WebSocket event frames compatible.

### Phase 2. ProjectionStore Skeleton

- Introduce projection structs for session snapshot, agent runtime, transcript page, workflow inspection, provider ledger, and health.
- Populate projections from existing `DaemonApp` state first.
- Add projection version and last-applied event metadata.

### Phase 3. CommandRouter Facade

- Route the first migrated commands through `CommandRouter`.
- The router may call existing `DaemonApp` methods during this phase, but request handlers no longer decide mutation ownership directly.
- Command accept/reject events must be emitted consistently.

### Phase 4. SessionActor and AgentActor

- Move session lifecycle/focus/attachment ownership into `SessionActor`.
- Move prompt queues, per-agent prompt states, and provider-run binding into `AgentActor`.
- Preserve the multi-agent invariant that focus does not park/resume/terminate another live run.

Current status: session lifecycle/focus/resize/end/delete behavior has been consolidated behind `KernelSessionService`, and prompt submit/cancel/complete/queue-advance behavior has been consolidated behind `KernelAgentService`. `SessionRuntime` and `AgentRuntime` now provide bounded per-session/per-agent mailboxes for responsiveness-critical command admission, session mailboxes are deregistered after successful end/delete, and focused-agent routing has its first session-owned projection. The mailbox workers still delegate most mutation through the compatibility services, so true actor-owned state is not complete until prompt queues, per-agent prompt states, and compatibility session fields stop requiring shared `DaemonApp` access.

### Phase 5. ProviderRunActor and Output Fanout

- Move provider event ingestion and provider-run liveness into `ProviderRunActor`.
- Make output fanout asynchronous and bounded.
- Ensure provider output can continue while a client is slow or disconnected.

### Phase 6. WorkflowRunActor and CapabilityExecutor

- Move workflow run progression out of shared app mutation and into `WorkflowRunActor`.
- Move file/tree/screenshot/shell/MCP capability jobs behind `CapabilityExecutor`.
- Connect capability progress/results to events and projections.

### Phase 7. WorkspaceCoordinator and RelayRuntime

- Add worktree/file/port claim enforcement.
- Move relay registration, remote subscriptions, and relay background I/O behind `RelayRuntime`.
- Validate that relay remains a transport/runtime member, not a workspace authority.

### Phase 8. Remove Hot-Path `DaemonApp` Mutex

- Block new hot-path handlers from taking `Arc<Mutex<DaemonApp>>`.
- Keep `DaemonApp` only as bootstrap/test compatibility until callers are migrated.
- Delete compatibility facade once tests and live drills pass without it.

## Required Tests and Drills

Unit and integration tests:

- duplicate `command_id` is idempotent
- invalid commands reject without mutation
- command ordering is preserved within one agent
- prompt submit and cancel do not wait behind history reads
- focus changes do not disturb another agent's live provider run
- provider output fanout does not block provider ingestion
- projection `last_event_id` advances with applied events
- replay from retained cursor returns missing events
- replay from expired cursor returns replay gap plus fresh projection
- slow subscriber disconnects without blocking other subscribers
- background capability queue applies backpressure
- worktree collision rejects concurrent code-writing claims

Live drills:

- slow `GetSessionHistory` while prompt submit still acknowledges immediately
- provider catalog discovery hangs while focus/cancel/resize still work
- one subscriber stops reading while another keeps streaming output
- reconnect during active streaming catches up without duplicate transcript lines
- reconnect after replay gap triggers projection resync
- multi-agent focus cycling while both agents are working preserves both liveness states
- workflow node dispatch refuses conflicting worktree claims
- relay background disconnect does not affect local session commands

## Exit Criteria

- Interactive commands do not wait behind history reads, capability jobs, provider discovery, provider output fanout, relay background work, or slow subscribers.
- The CLI can boot from projections and reconcile pushed events without synchronous whole-session refreshes on the hot path.
- Event replay has explicit retained-window and replay-gap behavior.
- Actor ownership is enforced by APIs, not convention.
- Worktree/collision coordination exists as a kernel-owned boundary.
- `Arc<Mutex<DaemonApp>>` is gone from hot request paths.
- Relay can build on the same command/event/projection model without becoming workspace authority.
