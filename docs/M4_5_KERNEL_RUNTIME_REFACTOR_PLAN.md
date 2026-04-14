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

The daemon is no longer just the old request-handler surface with new names. Provider-run structured submit/cancel/poll now runs through per-run actors, command-id retries fan out instead of double-dispatching, slow consumers and inbound request bursts have bounded handling, session lifecycle/focus/resize behavior is collected under `KernelSessionService`, and prompt submit/cancel/complete/queue-advance lifecycle behavior is collected under `KernelAgentService`. Prompt submit/cancel commands now enter per-agent mailboxes, session attach/detach/focus/cycle/resize/end/delete commands enter per-session mailboxes instead of the generic interactive queue, and the session runtime publishes a focused-agent projection used by agent routing once focus is warm. Agent lifecycle responses that change focus refresh the same projection. The router also has the first projection stores for warmed `GetSessionState`, `ListSessions`, successful `ResolveSession`, `GetSessionHistory`, `GetProviderRun`, `ListProviderProcesses`, and `GetProviderCatalog` reads; list responses hydrate per-session state entries; and prompt lifecycle mutations now publish session snapshots into the shared session projection.

Important caveat: the implementation still has a compatibility `DaemonApp`, and several hot paths still pass through shared app state while they migrate. M4.5 is in progress, not complete.

Current event replay is a bounded in-memory recent-event buffer. It supports reconnect-friendly local behavior, but it is not yet a durable event log.

Current multi-agent reliability work has already established one important invariant: focus is UI state, while prompt execution and provider-run liveness are per-agent. M4.5 must preserve that invariant by treating session-global active prompt/run fields as compatibility projections, not as ownership.

## Implementation Status

Status as of 2026-04-13:

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
- Landed: `KernelSessionService` now owns public session creation/default-agent bootstrap plus attach, detach, end, delete-by-ref, focus/cycle, and terminal resize behavior behind the public `DaemonApp` compatibility methods.
- Landed: `KernelAgentService` now owns prompt submit, kernel submit acknowledgement/dispatch preparation, cancel, runtime cancel, completion, queue advancement, and cancellation finalization behind the public `DaemonApp` compatibility methods.
- Landed: `AgentRuntime` per-agent command mailboxes for prompt submit/cancel admission, so agent-scoped prompt commands are not rejected behind unrelated generic interactive work.
- Landed: `SessionRuntime` command mailboxes for public session creation plus attach/detach/focus/cycle/resize/end/delete admission, so session-scoped UI and lifecycle commands are isolated from the generic interactive queue and from other sessions.
- Landed: session mailbox cleanup on successful end/delete, so closed session lanes do not stay registered indefinitely.
- Landed: `SessionRuntime` projection-backed lane resolution. Delete-by-ref and detach-by-attachment can resolve the target session from warmed projected session state before falling back to the compatibility app lock.
- Landed: Phase 8 `SessionRuntime` missing delete/detach resolution now returns `SessionNotFound`/`AttachmentNotFound` from a warmed session-list projection instead of taking the compatibility app lock just to confirm absence.
- Landed: `SessionRuntime` now publishes session and agent-runtime projections for attach/detach/focus/cycle/resize command results from inside the session mailbox owner, removing router-side post-command compatibility snapshots for those session UI/lifecycle commands.
- Landed: `DaemonHealthProjection` with session/agent mailbox queue snapshots, provider runtime operation-lane pressure, session projection counts, active/queued prompt counts, and provider-catalog cache status exposed through `GetDaemonHealth`.
- Landed: `AgentRuntimeProjectionStore` baseline. Session projection refreshes now materialize per-agent active/queued prompt read models, direct agent/session-scoped projection reads are available, and daemon health uses those agent-runtime prompt counts as the canonical prompt-work health source while mirroring them into legacy session-projection prompt-count fields for wire compatibility.
- Landed: agent-runtime prompt projections now include the front queued prompt per agent, and local/remote queue advancement uses that projected candidate before falling back to compatibility session queue reads.
- Landed: detach publishes refreshed session/agent-runtime projections after attachment-owned prompt cleanup, so queue-front projections are not left stale before advancement.
- Landed: provider idle settlement uses the agent-runtime active-prompt projection before compatibility session inspection when choosing complete/cancel/clear settlement behavior.
- Landed: `AgentRuntime` mailbox workers now carry the shared agent-runtime projection store and publish per-agent prompt projection updates from prompt submit/cancel execution.
- Landed: kernel `CompletePrompt` now routes through the per-agent mailbox, resolves the active prompt owner from warmed session projection before falling back to compatibility session state, and updates the agent-runtime projection from the mailbox worker after completion.
- Landed: `CompletePrompt` owner resolution now uses the agent-runtime active-prompt projection before session snapshots, removing another compatibility-store fallback when the per-agent prompt read model is warm.
- Landed: kernel `CancelActivePrompt` now uses the same agent-runtime active-prompt owner projection before session snapshots when routing into the per-agent mailbox.
- Landed: router response refresh no longer double-publishes agent-runtime projection state for `PromptSubmitted`; submit projection publication comes from the agent mailbox worker.
- Landed: agent mailbox workers now publish both session-state and agent-runtime projections for submit/cancel/complete prompt lifecycle mutations, so prompt submit no longer depends on a router-side session projection refresh.
- Landed: `AgentRuntimeProjectionStore` is now the single warm prompt-state read model during the compatibility migration. The temporary `AgentRuntimePromptStateStore` shadow was removed after review because it duplicated active/queued prompt ownership and could diverge from session snapshots observed through provider-output or workflow paths.
- Landed: per-agent prompt mailbox results now refresh the shared agent-runtime projection and session projection from the authoritative session returned by the mutation boundary. Router-observed session/list snapshots still refresh the same projection store for non-agent paths.
- Landed: prompt submit now enters the per-agent mailbox before lifecycle mutation, but `Start` vs `Queue` admission is decided by `PromptStateOwner`, not by a projection. Warmed agent-runtime projections still provide routing, queue-front previews, and read-model state, but stale projection data cannot force prompt admission.
- Landed: prompt submit now creates the prepared `PromptQueueItem` in the agent mailbox path before compatibility mutation, including the reserved prompt id, target agent, source attachment, body, and attachments. `KernelAgentService` mirrors that exact prepared prompt into compatibility session state for local and remote agents instead of creating a second prompt object.
- Landed: `AgentRuntimeProjectionStore` retains the queue front and queued count needed for warmed submit/completion routing. `PromptStateOwner` is now the kernel write owner for full queued-prompt state, and the projection is a read model refreshed from the compatibility mirror after owner mutations.
- Landed: `PromptStateOwner` is now a cloneable kernel service injected into `AgentRuntime` instead of a private `DaemonApp` sidecar. Warm active-prompt routing and complete queue-front preview can read the owner through the agent runtime when the session projection is available, so stale compatibility session prompt mirrors do not force an app-lock fallback.
- Landed: agent-mailbox prompt completion now consumes the session projection published by the prompt lifecycle service instead of taking a second compatibility session snapshot under the app lock after completion mutation.
- Landed: prompt completion now carries an `AgentRuntime` queue-front preview into the agent mailbox and reconciles compatibility advancement against that preview. This keeps queue advancement behavior unchanged while establishing the actor-owned input needed to flip advancement out of compatibility state.
- Landed: kernel prompt completion now passes the `AgentRuntime` queue-front preview into `KernelAgentService`, and local/remote compatibility queue advancement validates that the compatibility queue front matches the actor-owned preview before dispatching the next prompt.
- Landed: compatibility prompt queue activation now has an explicit expected-prompt path that removes the started prompt by id and refreshes projected queue-front/count fields, rather than relying on a blind pop-front after a separate pre-check.
- Landed: compatibility queue advancement now has an expected-prompt activation path. When the agent runtime supplies the queue-front preview, the session mirror validates and activates that exact prompt id atomically instead of doing a blind pop after a separate pre-check.
- Landed: agent mailbox prompt lifecycle handling now publishes `AgentRuntimeProjection` directly from the authoritative session returned by submit/cancel/complete. Session snapshots from non-agent paths refresh the same projection store, so there is no second prompt-state cache to reconcile.
- Landed: queue advancement now prefers the agent-runtime expected queued prompt as the dispatch candidate when the mailbox supplies one, using compatibility projection/session reads only as a fallback for legacy/background paths. Compatibility state still validates and mirrors the selected prompt id before activation.
- Landed: legacy/background queue-drain entrypoints now consult the warmed agent-runtime projection for the expected queued prompt before falling back to compatibility queue state, so provider-run startup and attachment cleanup paths follow the same actor-owned candidate path as prompt completion when projections are warm.
- Landed: `RuntimeSession` prompt mirror updates are now concentrated behind `PromptRuntimeState`, which maintains per-agent active/queued prompt state plus the legacy session-level active prompt, queued prompt, and scheduler projections for compatibility. The struct is flattened for serialization, so the wire shape stays compatible while prompt mutation authority lives in the kernel prompt owner.
- Landed: `PromptRuntimeState` now lives in `session/prompt_runtime.rs` as a dedicated compatibility mirror boundary. `RuntimeSession` flattens and forwards it for wire compatibility, while `PromptStateOwner` owns prompt lifecycle mutation.
- Landed: the unused direct complete-and-auto-advance prompt mutation API was removed from `RuntimeSession`/`PromptRuntimeState`. Prompt completion now stays on the kernel lifecycle path that completes the active prompt, validates the agent-runtime queue-front preview, and then advances the queued prompt explicitly.
- Landed: direct compatibility complete/cancel paths now resolve the active prompt owner with the same focused-active-or-single-active rule as the agent runtime, so focus no longer has to equal execution ownership when only one non-focused agent is running.
- Landed: `RuntimeSession` prompt mutators are now session-module-private, and provider dispatch failure cleanup routes through `KernelAgentService` instead of directly mutating `SessionService`. Non-session modules can still inspect compatibility prompt projections, but they can no longer call the underlying `RuntimeSession` prompt mutation helpers directly.
- Landed: Phase 8 prompt-submit routing can resolve a missing target agent from the single-agent `AgentRuntimeProjection` before falling back to the compatibility app lock. This removes another hot-path lock for warmed single-agent sessions, including sessions whose session projection is not yet warm.
- Landed: Phase 8 prompt cancel/complete owner resolution now returns `NoActivePrompt` from warmed session/runtime projections instead of taking the compatibility app lock just to confirm there is no active prompt.
- Landed: Phase 8 prompt submit/cancel/complete routing now returns `SessionNotFound` from a warmed session-list projection for missing sessions instead of taking the compatibility app lock after runtime/session prompt projections are empty.
- Landed: Phase 8 prompt submit target-agent validation now rejects projected `AgentNotInSession` cases before entering the agent mailbox or compatibility app lock when the session projection is warm.
- Landed: provider-run exit reconciliation no longer completes/cancels active prompts by mutating session prompt state directly and then separately draining the queue. It now delegates prompt settlement to the kernel prompt lifecycle service, preserving the runtime/projection-guided queue advancement path for unexpected provider exits.
- Landed: terminal output fanout now enforces a bounded pending-output backlog per attachment. Slow or disconnected output consumers cannot grow the in-memory terminal stream without bound; history remains the durable transcript source.
- Landed: Phase 8 daemon health reads terminal stream pressure from a cloned terminal health store instead of locking the compatibility app-owned terminal stream.
- Landed: Phase 8 missing session-scoped inspection/history reads now return `SessionNotFound` from warmed session-list projections instead of falling back to the compatibility app lock.
- Landed: Phase 8 relay status, remote-machine discovery, remote-kernel discovery, and provider command catalog reads now use router-owned config/projection data instead of taking the compatibility app lock.
- Landed: Phase 8 provider catalog reads now load and refresh the router-owned provider catalog projection from the config projection instead of taking the compatibility app lock on cold catalog paths.
- Landed: Phase 8 provider-launch pending guards now clear from session/provider-run projections when a launch has settled, so warmed session-state reads do not fall back to the compatibility app lock only because a stale launch guard remains.
- Landed: Phase 8 provider-launch pending cleanup no longer waits for the compatibility app lock when projections are cold. Cold cleanup leaves the guard in place for a later projection-backed refresh instead of delaying the current response.
- Landed: Phase 8 agent spawn/destroy now publish session projections inside the mutation boundary, and router-side agent lifecycle refresh consumes the published projection instead of taking a second compatibility app-lock snapshot.
- Landed: Phase 8 session config updates now enter the `SessionRuntime` mailbox and publish refreshed session projections from the session-command boundary, so post-update state reads remain projection-first without a compatibility app-lock read.
- Landed: Phase 8 session config update validation now rejects warmed missing/mis-scoped attachment cases from session projections before taking the compatibility app lock.
- Landed: Phase 8 session alias updates now enter the `SessionRuntime` mailbox and publish refreshed session projections, so alias resolution can use the projection immediately after mutation without taking the compatibility app lock.
- Landed: Phase 8 session mailbox workers now publish session-bearing create/config/alias/end responses directly and remove deleted-session projections at the mailbox boundary, avoiding redundant compatibility snapshots when the response already carries authoritative session state.
- Landed: Phase 8 runtime notice polling is now classified as an interactive session command and enters the `SessionRuntime` mailbox, keeping notice drains ordered with same-session config/lifecycle commands instead of the generic compatibility path.
- Landed: Phase 8 terminal output polling now uses warmed session projections for missing-session and missing-attachment rejection before falling back to the compatibility pump path, establishing the first projection-aware seam ahead of provider-runtime/terminal-store ownership extraction.
- Landed: Phase 8 terminal stream state now sits behind a cloneable `TerminalStreamStore` shared by `DaemonApp` and the router. `PumpTerminalOutput` can drain buffered output without the app lock when the warmed session projection shows no active provider run, while active-run pumping still falls back to compatibility provider I/O.
- Landed: Phase 8 active-run `PumpTerminalOutput` now enters the provider-run operation lane before invoking the compatibility provider pump, then drains the shared terminal stream store. Provider I/O is still implemented by the compatibility app path, but command admission is now serialized at the provider-run runtime boundary.
- Landed: Phase 8 introduced a `ProviderOutputPump` seam for active provider output polling. `DaemonApp` still supplies compatibility-owned provider, PTY, terminal fanout, and session dependencies, but the pump behavior now sits behind an explicit boundary that can be migrated toward provider-runtime-owned state incrementally.
- Landed: Phase 8 runtime notice polling now validates attachment/session ownership inside the `SessionRuntime` boundary but drains notices through the shared `TerminalStreamStore` instead of reaching through `DaemonApp` terminal state.
- Landed: Phase 8 runtime notice polling now uses warmed session projections inside the `SessionRuntime` worker, so valid notice drains and warmed attachment absence errors no longer wait for the compatibility app lock.
- Landed: Phase 8 active-run terminal output polling now skips compatibility provider pumping when warmed provider-run projection shows the active run is parked or ended, draining buffered terminal output directly. Compatibility pump paths refresh session/agent projections inside the mutation boundary instead of taking a second post-response app lock.
- Landed: Phase 8 inactive-run terminal output polling now resolves directly from session/provider-run projections before entering the fallback executor, and provider liveness reconciliation republishes already-ended provider-run projections when the provider pump observes them.
- Landed: Phase 8 terminal resize now uses warmed session projections inside the `SessionRuntime` worker for missing-session and no-active-provider-run rejection, avoiding compatibility app-lock access for those absence cases.
- Landed: Phase 8 session attach/focus/cycle validation now rejects warmed missing-session cases, and focus rejects warmed missing-agent-in-session cases, inside the `SessionRuntime` worker without compatibility app-lock access.
- Landed: Phase 8 direct session-lane resolution now rejects warmed missing attach/focus/cycle/resize/alias/end requests before creating a per-session lane, avoiding orphan lanes for projected-absent sessions.
- Landed: Phase 8 attachment-scoped session lane resolution now rejects warmed missing/mis-scoped runtime-notice and config-update attachments before creating a per-session lane.
- Landed: Phase 8 session alias/end validation now rejects warmed missing-session cases inside the `SessionRuntime` worker without compatibility app-lock access.
- Landed: Phase 8 provider-process teardown now refreshes affected session and agent-runtime projections inside the teardown mutation boundary, so post-teardown session reads observe cleared active provider runs without a second compatibility app-lock refresh.
- Landed: Phase 8 async provider-launch completion/failure now publishes session projections when active provider-run state changes, so warmed session reads observe accepted-to-running and failed-launch state transitions without a compatibility app-lock refresh.
- Landed: structured provider output polling now drains all finished provider-run actor poll jobs on each pump pass. Non-requested run output is applied into the terminal fanout buffer for its recipients and retained in a per-provider-run return buffer so a later direct `pump_provider_output(run_id)` call still receives that run's records.
- Landed: multi-run structured output regression coverage now proves a pump for one provider run applies already-finished output jobs from another run into the terminal fanout buffer without returning that background output as requested-run output.
- Landed: provider-run actor command mailboxes now use bounded per-run queues and non-blocking enqueue. Submit/abort/poll/sync/terminate commands cannot grow an unbounded provider-run worker backlog under repeated polling or lifecycle pressure.
- Landed: daemon health now includes provider-run actor enqueue counters, including accepted commands and enqueue rejections, so provider runtime pressure is visible separately from the per-run operation-lane occupancy snapshot.
- Landed: structured provider submit/abort/output-poll/selection-sync enqueue failures now propagate as daemon errors instead of being logged and swallowed. Prompt dispatch cleanup can now run when the provider actor does not actually accept a structured prompt job, and structured output pumps now surface output-poll enqueue failure to callers.
- Landed: daemon health now includes terminal stream backlog pressure, including pending output/notice/completion record counts, the per-attachment pending-output limit, and trimmed pending-output recipient deliveries.
- Landed: provider-run exit prompt settlement is isolated behind a dedicated prompt-lifecycle boundary helper. Unexpected provider exits now choose cancellation finalization, completion, or idle sync in one place before Phase 5 moves more liveness ownership into `ProviderRunActor`.
- Landed: provider-run exit prompt settlement now uses an explicit settlement decision object, separating the active-prompt-status decision from the compatibility mutations it currently triggers.
- Landed: provider-run liveness reconciliation now lives behind the provider runtime boundary. `DaemonApp` still observes PTY process state during compatibility migration, but provider state transitions, active-run cleanup, runtime cleanup, and ended/external/still-running/newly-ended classification are no longer open-coded in prompt lifecycle handling.
- Landed: `WorkflowRuntime` baseline with bounded per-session workflow command lanes. Workflow creation/graph/run/watchdog/ack/validation/queue commands now enter a workflow-owned mailbox before hitting compatibility mutation, and daemon health exposes workflow lane pressure separately from session/agent/provider lanes.
- Landed: workflow command lane cleanup on session end/delete, so per-session workflow mailboxes do not stay registered after their owning session is gone.
- Landed: workflow command lane resolution now rejects warmed missing-session workflow mutations from `SessionStateProjectionStore` without creating a lane or waiting on the compatibility app lock, and workflow workers refresh projections from session-bearing workflow responses before falling back to a compatibility snapshot.
- Landed: workflow runtime-tool projection refresh. Direct MCP/relay runtime-tool calls now republish session and agent-runtime projections after recording tool-call state, so acknowledgements and output submissions do not leave warmed workflow inspection reads stale when they bypass the router workflow lane.
- Landed: capability executor health counters for submitted/running/completed/failed/rejected/join-error jobs, plus the executor concurrency limit and available permits. `GetDaemonHealth` now exposes capability job pressure separately from actor mailbox pressure.
- Landed: workflow prompt start/completion/cancellation, workflow provider-run ensure, workflow resume, workflow-console runtime tools, and blocked-claim retry entrypoints are now isolated behind the app workflow-runtime boundary, so prompt lifecycle, transport, runtime-tool, and local API callers no longer invoke scheduler runtime progression functions directly.
- Landed: `FocusedAgentProjection` shared by `SessionRuntime`, `AgentRuntime`, and router-side agent lifecycle refreshes, so focus changes captured by the session mailbox or local agent spawn/destroy responses let untargeted prompt submit/cancel route to the focused agent without synchronously taking the compatibility app lock for focus lookup.
- Landed: Phase 8 router-side focus refresh now runs after session projection refresh and reads the warmed session projection first, avoiding a second compatibility app lock after agent lifecycle commands have already refreshed session state.
- Landed: `SessionStateProjectionStore` for warmed session/list/resolve/prompt-state snapshots. `GetSessionState`, successful `ResolveSession`, and warmed `ListSessions` can return projected session data without taking the compatibility app lock after the projection has been warmed by list responses, prompt/session/agent/workflow responses, prompt lifecycle publications, or transitional post-mutation snapshots. List warm-up now hydrates per-session projection entries, so follow-up state and successful resolve reads can stay projection-first without requiring a separate session-state read.
- Landed: Phase 8 missing `GetSessionState` and `ResolveSession` reads now return `SessionNotFound`/ambiguity from the warmed session-list projection instead of falling back to the compatibility app lock.
- Landed: `AgentRuntime` focus fallback through the warmed session projection. Untargeted prompt submit/cancel now checks projected focused-agent state before falling back to the compatibility app lock when the dedicated focus projection is cold.
- Landed: `SessionHistoryProjectionStore` for warmed transcript history. `GetSessionHistory` can use a warmed session snapshot to load disk history without taking the compatibility app lock, then serve repeated warmed transcript reads from memory while successful appends keep the warmed projection current.
- Landed: `ProviderRunProjectionStore` for warmed provider-run snapshots. `GetProviderRun` can return without taking the compatibility app lock after launch/read warm-up, and start/finish/fail/park/resume/end lifecycle updates refresh the shared projection.
- Landed: `ProviderProcessProjectionStore` for warmed process-list snapshots. `ListProviderProcesses` can return without taking the compatibility app lock after warm-up, and provider-run/session lifecycle changes invalidate the snapshot so teardown-safety metadata is recomputed before reuse.
- Landed: prompt-state projection publication. Prompt submit, completion, cancellation, dispatch failure, and queue advancement now publish updated session snapshots into the shared projection, so warmed `GetSessionState` reflects prompt lifecycle transitions without a compatibility-store read.
- Landed: prompt complete/cancel projection publication is now authoritative for router refresh. The router no longer takes a redundant post-response session snapshot for those prompt lifecycle commands.
- Landed: router-side session projection refreshes are trimmed for non-state terminal control commands. `PollRuntimeNotices` and `ResizeTerminal` no longer reacquire the compatibility app store for a session snapshot; `PumpTerminalOutput` still refreshes because output polling can change prompt lifecycle state.
- Landed: `ProviderCatalogProjectionStore` for TTL-bound warmed provider catalog snapshots. `GetProviderCatalog` can return without taking the compatibility app lock after warm-up, and provider logout/configuration changes invalidate the projection.
- Landed: projection correctness hardening. Provider-process snapshots are canonical rather than request-scoped, teardown refreshes affected process/run projections, warmed OpenCode provider-run reads preserve selection-sync side effects, relay reconfiguration invalidates provider-catalog projection state, and agent command lanes are cleaned up on agent/session removal.
- Landed: transport health projection. Kernel websocket slow-consumer closes, outgoing queue overflows, inbound overload rejections, replay gaps, request/event counts, active connections, and active subscriptions are now surfaced through `GetDaemonHealth`.
- Landed: workspace coordination health baseline. `GetDaemonHealth` now reports active worktree claims and same-workspace worktree collisions from the warmed session projection without taking the compatibility app lock.
- Landed: projection invariant health. `GetDaemonHealth` now reports checked session/agent projection counts and any mismatch between warmed session prompt state and the agent-runtime prompt read model, making stale projection refresh paths observable instead of latent.
- Landed: session inspection reads from the warmed session projection. `ListAgents`, `ListWorkflows`, `ResolveWorkflow`, `ListWorkflowRuns`, `GetWorkflowRun`, `ListWorkflowWatchdogs`, and `ListQueuedWorkflowLaunches` can now return from projected session state without taking the compatibility app lock after the session projection is warm.
- Landed: initial `WorkspaceCoordinator` enforcement. Explicit file-writing capabilities (`EditFile` and `StoreTransferredFile`) acquire scoped worktree write claims, reject overlapping writes in the same workspace/worktree with a retryable workspace-claim conflict, expose active operation claims in daemon health, and release claims when the operation completes.
- Landed: provider prompt lifecycle worktree claims. Active local provider prompts acquire a provider-prompt operation claim before dispatch, release it through the prompt activity cleanup path on completion/cancellation/dispatch failure/session cleanup, and reject cross-session prompt dispatch onto the same workspace/worktree while preserving same-session shared-worktree prompt flows.
- Landed: workspace claims as a scheduler primitive. Claims now carry `read`/`write` mode metadata, use normalized real worktree keys where possible, and workflow node dispatch attempts an exclusive `workflow_node_dispatch` write claim before submitting provider work. Workflow nodes that hit a busy worktree move to `BlockedOnWorkspaceClaim` and retry when prompt/workflow claims release instead of failing the workflow.
- Landed: local prompt submit keeps provider-prompt claim admission synchronous so workspace conflicts still fail fast, then moves local provider prompt writes/enqueues into the spawned provider-run operation dispatch. The agent mailbox can acknowledge the owner-backed prompt mutation without performing PTY writes or provider actor enqueue work inline.
- Landed: kernel prompt submit now defers user-prompt history appends and remote relay prompt dispatch out of the agent-mailbox acknowledgement path. History appends run through a spawned blocking append that refreshes the warmed history projection after successful persistence, and remote relay prompt failures cancel the acknowledged active prompt, refresh projections, and publish a notice.
- Landed: session end/delete now clears the shared terminal stream store for that session from the `SessionRuntime` boundary. Pending output, notices, completion records, and recorded terminal input no longer survive after the owning session is gone.
- Landed: relay-client daemon/workflow requests now enter through `CommandRouter` as `KernelCommandSource::RelayClient` instead of calling `DaemonApp::handle_local_request` directly. Proxied relay clients now share the same actor admission, projection refresh, and overload behavior as local IPC/kernel transport requests, including workflow output validation and acknowledgement requests that previously died at the relay transport gate.
- Landed: the legacy generic interactive lane no longer falls through to `DaemonApp::handle_local_request`. It now executes only recognized session/agent commands through their explicit runtime handlers and immediately rejects unsupported commands without waiting for the compatibility app lock.
- Landed: agent lifecycle mutations now route through the session runtime lane. `SpawnAgent` and `DestroyAgent` are interactive session-scoped commands, share the explicit agent request handler with local IPC, publish session/focus projections from the runtime path, and no longer enter the normal local compatibility fallback.
- Landed: runtime MCP now uses the kernel `CommandRouter` as its transport boundary. The MCP HTTP server binds from the router-owned config projection, authenticated local workflow runtime tools dispatch through the router, and forwarded relay workflow runtime tools also enter through the router instead of locking `DaemonApp` directly from transport handlers.
- Landed: post-slice live drills passed on 2026-04-14. Coverage included the daemon smoke harness, focused-agent multi-agent prompt routing, shared-endpoint multi-agent prompt routing, workflow progression without terminal pumps, downstream workflow scheduling, join-node scheduling, workflow workspace-claim retry, and CLI workflow graph/outline drill catalogs.

Still open:

- move `KernelSessionService` state out of the compatibility facade and into the new `SessionRuntime` mailbox owner
- finish moving the remaining prompt side effects out of the compatibility app lock now that provider writes/enqueues, user history append, and remote relay prompt dispatch are off the agent-mailbox acknowledgement path; claim storage and compatibility mirroring still need ownership boundaries
- broaden actor-owned projections beyond focused-agent routing and warmed session/list/history/provider-run/process/prompt-state/provider-catalog snapshots so remaining provider/read models can be served without compatibility-store reads on the hot path
- remove remaining actor-worker mutation paths that require `Arc<Mutex<DaemonApp>>` as the compatibility mirror
- keep the current `WorkspaceCoordinator` enforcement at coarse worktree safety/scheduler scope while actor/projection ownership is completed; deeper file-level scopes, port claims, sandbox enforcement, and transactional patch/rebase coordination are intentionally deferred to the final I/O-coordination slice

## A+ Completion Plan

The A+ bar is not only green tests. The kernel runtime should have single ownership for mutable state, projection-first reads, explicit overload behavior, and enough health signal to debug stalls without reading code.

Order of work:

1. Stabilize the current branch: keep daemon tests green, remove duplicate prompt-state components, and make provider actor enqueue failure observable and retryable.
2. Finish prompt ownership: `PromptStateOwner` now owns active/queued prompt lifecycle mutation, is shared with `AgentRuntime`, mirrors into `PromptRuntimeState`, and local provider writes/enqueues, prompt history appends, and remote relay prompt dispatch run after acknowledgement through spawned side-effect paths; next remove the remaining hot app-lock access around claim storage and compatibility mirroring side effects.
3. Finish session ownership: move remaining lifecycle, attachment, config, alias, focus, resize, end, and delete state behind `SessionRuntime`; session deletion now cleans terminal stream buffers from the runtime boundary, and must continue consolidating agent lanes, workflow lanes, provider runs, claims, and projections through one ordered path.
4. Formalize projection correctness: centralize projection refresh helpers, define which authoritative mutation refreshes which projection, and add stale-state regression tests for provider output, provider teardown, workflow progression, session delete, agent destroy, prompt cancel, prompt complete, and daemon-health projection invariant drift.
5. Harden workflow runtime: move workflow progression, blocked-claim retry, node completion, watchdogs, and queued launches out of direct compatibility mutation paths and behind workflow-owned lanes.
6. Harden provider and terminal runtime: ensure provider I/O does not hold hot app locks, give structured submit/abort/output-poll jobs explicit lifecycle states, and bound notice/completion buffers with health counters alongside output buffers.
7. Remove hot `DaemonApp` dependencies: audit every remaining `Arc<Mutex<DaemonApp>>` request path, classify it as bootstrap, compatibility, or hot-path blocker, and move blockers behind actors/projections.
8. Lock docs and invariants: keep README, architecture, roadmap, protocol, task board, and progress log aligned after each slice; enforce the implementation-invariants checklist for ownership, projection refresh, cleanup, overload behavior, health, and tests in [IMPLEMENTATION_INVARIANTS.md](/Users/miguel/arroba/docs/ops/IMPLEMENTATION_INVARIANTS.md).
9. Return to final I/O coordination last: decide sandbox/overlay/coordinator-owned patch semantics, same-session behavior, file/port resource scopes, and transactional rebase/repair workflows only after actor/projection ownership is stable.

### Current Hot-Path Lock Audit

Status as of 2026-04-13:

The remaining `Arc<Mutex<DaemonApp>>` use falls into these buckets:

| Area | Current use | M4.5 treatment |
|------|-------------|----------------|
| `kernel/router.rs` | compatibility fallback reads, projection refresh snapshots, and provider launch/process/catalog cold paths; the generic interactive request fallback is now closed | keep cold/background reads bounded; remove fallback reads from warmed interactive paths before exit |
| `kernel/session_actor.rs` | per-session mailboxes serialize requests but still execute lifecycle/focus/resize mutations through compatibility app state | next ownership slice: make `SessionRuntime` own normal session UI/lifecycle state and mirror compatibility snapshots |
| `kernel/agent_actor.rs` | per-agent mailboxes serialize prompt submit/cancel/complete and share `PromptStateOwner` for active-owner and queue-front decisions; local provider dispatch, history append, and remote relay submit are now deferred side effects, while claim storage and mirror publication still enter `KernelAgentService` through compatibility app state | next ownership slice: split the remaining claim/mirror side effects so the app lock is not needed for normal owner-backed routing and admission |
| `kernel_transport.rs` and `transport/relay_client.rs` | bootstrapping, relay/presence/config lookups, encrypted request decrypt/encrypt, peer lease helpers, and subscription helpers still read app state; relay-client daemon/workflow requests plus local and forwarded runtime MCP tool calls now route through `CommandRouter` | keep relay as transport-owned background work; move registration/subscription/peer helper state behind `RelayRuntime` after session/agent ownership |
| `scheduler/runtime.rs` and `transport/runtime_tools.rs` | workflow progression, runtime tools, and preserved turn state still operate over shared app/session state, but the HTTP MCP transport no longer calls this state directly | introduce a workflow actor boundary after prompt ownership; do not broaden I/O coordination yet |
| `kernel/capability_executor.rs` | capability jobs use app state for session/attachment context and write-claim coordination | acceptable only because jobs are bounded/background; final arbitrary I/O enforcement is deferred |

New regression coverage locks in the current responsiveness contract while ownership continues moving:

- slow session history reads must not delay prompt submission
- slow provider catalog discovery must not delay focus, terminal resize, or prompt cancellation

These tests do not prove actor ownership is complete. They specifically prevent backsliding on the M4.5 exit criterion that user-visible interactive commands must stay responsive while slow read/background work is running.

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
| coarse worktree claims | `WorkspaceCoordinator` | Enforces visible collision prevention before file-writing capability/provider/workflow dispatch work without attempting final I/O conflict control. |
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

M4.5 introduces the runtime boundary for workspace coordination without trying to solve the full multi-agent I/O conflict problem yet.

The current claim system is a bounded kernel safety layer and scheduler signal. It prevents obvious overlapping worktree mutations that Arroba can see today, exposes active claims in health, and lets workflow scheduling block/retry instead of failing temporary contention. It is not the final conflict-control architecture for arbitrary agent harnesses, filesystem writes, patch transactions, or merge automation.

Minimum `WorkspaceCoordinator` responsibilities:

- allocate or validate `WorktreeAssignment`
- track active worktree claims now and leave file/port scopes for the final I/O-coordination design
- reject concurrent code-writing work in the same active worktree when claims conflict
- expose claim state in `DaemonHealthProjection` or a dedicated coordination projection
- release stale claims on provider-run settlement, cancellation, session delete, or explicit recovery

Current implementation:

- active session/worktree collisions are visible in `DaemonHealthProjection`
- explicit file-writing capability operations acquire scoped worktree write claims and reject overlapping writes in the same workspace/worktree
- active local provider prompts acquire provider-prompt worktree claims before dispatch, reject cross-session same-worktree prompt conflicts, and release claims through prompt completion, cancellation, dispatch failure, and session cleanup
- workflow node dispatch is claim-aware scheduler work: busy worktrees move nodes to `BlockedOnWorkspaceClaim`, preserve the prepared turn prompt, and retry after claim release
- claims carry explicit `read`/`write` mode metadata; current filesystem-mutating operations and provider/workflow prompt execution use write claims
- active operation claims are visible in `DaemonHealthProjection`
- file capability claims are released by scoped guards when the capability operation completes; provider prompt claims are held by the prompt lifecycle until the active prompt settles

Still open:

- define the final I/O-coordination model after the actor/projection refactor is complete
- decide how enforced mutation control works across harnesses Arroba does not fully control: OS/filesystem sandboxing, read-only canonical worktrees plus writable overlays, coordinator-owned patch application, harness permission gates, or an explicit fallback advisory mode
- file-level claim scopes, if they still make sense after the enforcement model is chosen
- port claim scopes
- transactional patch submission/rebase protocol, if Arroba chooses coordinator-owned canonical writes

Rules:

- Workflow node dispatch consults the coordinator before starting a code-writing node.
- Capability jobs that mutate files consult the coordinator.
- Provider-run placement records the assigned worktree in runtime state.
- Shared-session worktree mode remains allowed for single-agent or explicitly serialized work.
- Same-session shared worktrees must remain a product-supported collaboration mode. The current provider-prompt same-session allowance preserves that behavior; it should not be expanded into deeper I/O policy without a separate design review.
- Do not expand claims into file-level/port-level enforcement until `SessionRuntime`, `AgentRuntime`, workflow ownership, and projection-first reads no longer depend on hot `Arc<Mutex<DaemonApp>>` paths.

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
- Keep `PromptStateOwner` as the prompt lifecycle authority, continue moving prompt side effects behind `AgentRuntime`, and move provider-run binding out of compatibility session fields.
- Preserve the multi-agent invariant that focus does not park/resume/terminate another live run.

Current status: Phase 4 migration slices are complete for the M4.5 boundary. Public session creation/default-agent bootstrap and session lifecycle/focus/resize/end/delete behavior are consolidated behind `KernelSessionService`, and prompt submit/cancel/complete/queue-advance behavior is consolidated behind `KernelAgentService`. `SessionRuntime` and `AgentRuntime` provide bounded session/per-agent mailboxes for responsiveness-critical command admission, session mailboxes are deregistered after successful end/delete, focused-agent routing is projection-backed, queue-front previews are read from `PromptStateOwner` or the shared agent-runtime projection, and provider/background settlement paths route through the kernel prompt lifecycle service instead of directly mutating prompt queues. `PromptStateOwner` is now the shared write owner for per-agent active/queued prompt state; `PromptRuntimeState` remains the flattened compatibility mirror in `session/prompt_runtime.rs`.

### Phase 5. ProviderRunActor and Output Fanout

- Move provider event ingestion and provider-run liveness into `ProviderRunActor`.
- Make output fanout asynchronous and bounded.
- Ensure provider output can continue while a client is slow or disconnected.

Current status: Phase 5 is complete for the M4.5 boundary. Provider-run structured submit/abort/poll commands enter bounded per-run actor mailboxes, runtime-slot tombstones prevent stale runtime restoration after cleanup, finished structured output jobs are drained globally on each pump, terminal fanout is bounded per recipient, and daemon health exposes both provider-run actor enqueue pressure and terminal stream backlog pressure. Provider-run liveness reconciliation is now a provider-runtime boundary concern, while prompt settlement remains intentionally delegated to the kernel prompt lifecycle service so queue advancement still follows the agent-runtime projection path.

Provider-run worker decision: keep the current thread-per-provider-run mailbox worker for M4.5. This is intentionally conservative because Codex/OpenCode runtime calls can block and own non-`Send`/process-local state; moving to bounded Tokio workers would require auditing every provider runtime operation for async cancellation, blocking behavior, and runtime-state ownership. The professionalization requirement for M4.5 is bounded admission, health visibility, cleanup correctness, and tests around multi-run output fairness. A Tokio worker pool remains a post-M4.5 optimization only after provider runtime calls are wrapped behind explicit blocking boundaries.

### Phase 6. WorkflowRunActor and CapabilityExecutor

- Move workflow run progression out of shared app mutation and into `WorkflowRunActor`.
- Move file/tree/screenshot/shell/MCP capability jobs behind `CapabilityExecutor`.
- Connect capability progress/results to events and projections.

Current status: Phase 6 is complete for the M4.5 boundary. Workflow commands are admitted through bounded per-session `WorkflowRuntime` lanes and publish session/agent-runtime projections from the lane worker after compatibility mutation. Capability jobs execute behind `CapabilityExecutor`; health reports executor job pressure and the executor enforces a bounded concurrency limit with explicit overload rejections. Workflow prompt start/completion/cancellation, workflow provider-run ensure, workflow resume, workflow-console runtime tools, and blocked-claim retry callers now enter through app workflow-runtime methods instead of calling scheduler progression directly. The scheduler implementation still mutates compatibility session state internally; removing that compatibility store is part of Phase 8 hot-path ownership, not an additional Phase 6 gate.

### Phase 7. WorkspaceCoordinator and RelayRuntime

- Add coarse worktree claim enforcement as a kernel-owned boundary for visible mutating work.
- Defer file-level claims, port claims, harness sandboxing, and transactional mutation/rebase semantics until after hot-path actor/projection ownership is complete.
- Move relay registration, remote subscriptions, and relay background I/O behind `RelayRuntime`.
- Validate that relay remains a transport/runtime member, not a workspace authority.

### Phase 8. Remove Hot-Path `DaemonApp` Mutex

- Block new hot-path handlers from taking `Arc<Mutex<DaemonApp>>`.
- Keep `DaemonApp` only as bootstrap/test compatibility until callers are migrated.
- Delete compatibility facade once tests and live drills pass without it.

Current status: Phase 8 is in progress. Warmed prompt submit/cancel/complete routing, target validation, and missing-session rejection, prompt lifecycle mutation ownership, prompt-completion consumption of lifecycle-published projections, prompt-submit user-history append deferral, remote relay prompt dispatch deferral, relay-client daemon/workflow request routing through `CommandRouter`, session read/resolve/inspection/history paths, delete/detach lane resolution, direct session-lane absence rejection, attachment-scoped lane validation, workflow-lane missing-session rejection, explicit session/agent/workflow request handling through their runtime boundaries, closure of the legacy generic interactive fallback, session-scoped agent lifecycle routing through `SessionRuntime`, session attach/focus/cycle/alias/end absence validation, session response-borne projection refresh/removal, provider-process teardown projection refresh, async provider-launch session projection refresh, daemon health terminal pressure and projection-invariant drift reporting, relay status/discovery reads, provider catalog/command catalog reads, settled provider-launch guards, non-blocking cold provider-launch cleanup, agent lifecycle refresh, session config/alias updates, projected session-config attachment validation, runtime notice polling through the terminal store and warmed session projection, terminal output absence/no-active-run/inactive-run projection draining, terminal resize absence handling, active-run provider-lane pumping, provider liveness projection refresh from the provider pump, the provider output pump seam, workflow response-borne projection refresh, workflow runtime-tool projection refresh, and router-side focus refresh now prefer actor/runtime/session/config projections and avoid compatibility app-lock fallback in the covered success and absence cases. Session, agent, and workflow command logic now each have one compatibility handler under their runtime boundary; local API and actor workers delegate to those boundaries instead of carrying duplicate mutation components. Post-slice multi-agent and workflow live drills passed on 2026-04-14. The remaining Phase 8 work is compatibility-facade debt: command workers still use `DaemonApp` as the mutation mirror, and cold session/provider-auth/configuration paths still use compatibility state until their runtime-owned stores are separated. Do not delete the compatibility facade until that larger ownership flip is complete.

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
